#![no_std]
#![no_main]

//! plic — **中断面域**：外部中断的收与结（S 态 supervisor 域，一个线程）。
//!
//! ```text
//! 装会话（交一枚孔给父域，父域按名字认领）
//!   → 收配给（父域按同一张需求单推来记录，按 Slot 归位）
//!   → 接上控制器自报的每一条线
//!   → 循环：等铃 → claim 到空 → 按线静音 + complete → 应铃 → 到点把线放回去
//! ```
//!
//! # 它是怎么被叫醒的
//!
//! 内核只有一件关于外部中断的知识：**"有外部中断"**。它把这一件事记进一枚**门铃**，
//! 铃响即唤醒等在这枚铃上的域——就是本域。谁在响、是哪条线、该干什么，内核一概不知，
//! 故它只能摇铃，**claim 只能由本域做**：claim 是"领走"，谁领谁欠 `complete`，
//! 内核一领就进了数据面。
//!
//! # 配给怎么到手（"名字跟着门闩走"）
//!
//! ```text
//! 1  本域按会话协议装一条叫 records 的泊位 → 那枚孔落到父域表里
//! 2  父域按需求单把要的几样交出来，再把「名字 + 句柄」的记录推进那条通道
//! 3  本域解出记录，按需求单的 Slot 归位
//! ```
//!
//! **字节长什么样不在这里**（那是 [`pairing`]，与父域同一份）；本域只说"我要哪几格"。
//!
//! # 第一刀：还没有客户端
//!
//! 本域只做"收与结"：接上所有线、claim、complete、应铃。**没有**"名字 → 线号"的表，
//! 也没有投递给客户端那一段——那是第二刀。
//!
//! 本域**不读走设备里的字节**：`serial@10000000` 的接收字节归 console。不读 ⇒ 源头一直
//! 挂着电平，故每领一条线就把它**静音**（`priority = 0`），等铃静下来一拍再把线放回去。
//! 按线静音本来就是驱动域自己的细杠杆，第一刀正好把它走一遍。

extern crate alloc;
// 本包 lib 提供 `_start` + panic_handler；必须真的链接它，`use` 只带符号不算。
extern crate programs;

// 共享物住在 supervisor 目录里，由两个 bin 各自声明一次（见 `needs.rs` 头注）。
#[path = "../needs.rs"]
mod needs;
#[path = "../pairing.rs"]
mod pairing;

/// 设备侧（本域私有，同 `lib.rs` 的纪律：谁的设备谁自己带）。
mod plic;
mod uart;

use env::PieToken;
use env::wire::PAIR_LEN;
use protocol::session::Quay;
use runtime::core::bell::Bell;
use runtime::core::dock::Dock;
use runtime::env::debug;
use runtime::env::mail::{NolePie, PolePie};
use runtime::env::room::exit_with;
use runtime::env::unit as utask;

use crate::plic::{LINE_PRIORITY, Plic};

/// 与父域之间那条通道的名字（两端按它对位，不靠位置约定）。
const RECORDS: &str = "records";

/// 装泊位/等配给的期限（毫秒）。
const QUAY_MS: usize = 1000;

/// 有静音的线时等铃的上界（毫秒）。**只有这时才用节拍**——闲的时候本域真睡（见主循环）。
const IRQ_WAIT_MS: usize = 20;

/// 静音位图能记到第几条线（`u64` 一位一条）。
///
/// 越界的线不记账：当场放回即可（virt 上 `ndev` 是 32，这条够不着）。
const MUTE_BITS: u32 = 64;

/// 收记录用的缓冲容量：需求单几条就备几条（父域按**同一张单子**推）。
///
/// 发货方不必抄这个数（`pairing::pack` 按单子逐条走）；收货方要它，因为它得**备缓冲**。
const CAP: usize = PAIR_LEN * needs::PLIC.len();

/// 日志节流：前几次全打（那是"通了没有"的读数），之后每这么多条打一次。
const LOG_FIRST: usize = 16;
const LOG_EVERY: usize = 64;

/// 失败编号指"死在装配的哪一步"（本域是常驻的，正常退场码不作数）。
const E_SIRE: usize = 1;
const E_UP: usize = 2;
const E_GRANT: usize = 3;
const E_OPEN: usize = 4;
const E_TREE: usize = 5;
const E_BELL: usize = 6;

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    let Ok(slots) = boot() else {
        exit_with(E_GRANT)
    };

    // 开图 + 读树：控制器、本域的 context、要接的线。
    let Ok(plic_dock) = Dock::open(PolePie::from_token(slots[needs::Slot::Plic as usize].get()))
    else {
        exit_with(E_OPEN);
    };
    let Ok(dtb_dock) = Dock::open(PolePie::from_token(slots[needs::Slot::Dtb as usize].get()))
    else {
        exit_with(E_OPEN);
    };
    let Ok(src_dock) = Dock::open(PolePie::from_token(
        slots[needs::Slot::Source as usize].get(),
    )) else {
        exit_with(E_OPEN);
    };
    let Some((plic, lines)) = Plic::new(plic_dock.view(), dtb_dock.view()) else {
        exit_with(E_TREE);
    };
    say("plic: docks open");
    let bell = Bell::new(NolePie::from_token(slots[needs::Slot::Bell as usize].get()));

    for &line in &lines {
        plic.enable(line, LINE_PRIORITY);
    }
    // 测试源：把"收到字节就拉线"打开。**必须在 enable 之后**——控制器先就位，线再开闸。
    // 读走字节的仍是内核的调试面（本域只动中断使能这一位），故敲键不会丢给谁。
    uart::arm_rx(src_dock.view());
    say(&alloc::format!(
        "plic: ready ({} lines, ctx {})",
        lines.len(),
        plic.context()
    ));

    // 循环：等铃 → 领到空 → 逐条静音 + 结 → 应铃 → 到点把静音的放回去。
    //
    // **闲的时候真睡**：`muted == 0` 时 wait 传 `usize::MAX`（永久挂起），本域一次都不醒。
    // 只有手里还压着静音的线时才退回节拍——那是第一刀唯一能"放回"的手段（没有客户端会来
    // 说一声"我抽干了"）。第二刀把放回换成事件，这个节拍随之消失。
    let mut total = 0usize;
    let mut muted: u64 = 0;
    loop {
        // 铃是一位：闲时（`muted == 0`）即使中断此刻就到，`ring` 也会把本域唤醒——不会漏。
        let wait = if muted == 0 { usize::MAX } else { IRQ_WAIT_MS };
        match bell.wait(wait) {
            // 铃响：把这一轮该领的都领走。
            Ok(true) => {
                let mut got = 0usize;
                // 这一轮领到的第一条线——**日志必须报它**：不报线号就分不清"一次敲键
                // 一条线报了好几次"与"好几条线各报一次"。
                let mut first_line = 0u32;
                loop {
                    let line = plic.claim();
                    if line == 0 {
                        break;
                    }
                    if first_line == 0 {
                        first_line = line;
                    }
                    plic.disable(line);
                    if line < MUTE_BITS {
                        muted |= 1 << line;
                    } else {
                        // 记不下就当场放回：宁可让它再报一次，也不能把它忘在静音里。
                        plic.enable(line, LINE_PRIORITY);
                    }
                    plic.complete(line);
                    got += 1;
                }
                // 应铃：清掉那一位并让内核**立即**重开本 hart 的闸门。
                let _ = bell.hush();
                total += got;
                if total <= LOG_FIRST || total.is_multiple_of(LOG_EVERY) {
                    say(&alloc::format!(
                        "plic: irq #{total} (+{got}) line {first_line}"
                    ));
                }
            }
            // 到点：只把**真被静音过**的那几条放回去，放完就清零 ⇒ 下一轮如果没人再响，
            // 本域回到永久挂起。
            Ok(false) => {
                let mut bits = muted;
                muted = 0;
                while bits != 0 {
                    let line = bits.trailing_zeros();
                    bits &= bits - 1;
                    plic.enable(line, LINE_PRIORITY);
                }
            }
            // 铃死了（父域收摊）⇒ 本域也没事可做。
            Err(_) => exit_with(E_BELL),
        }
    }
}

/// 装配的前半：装会话 → 收配给 → 按 `Slot` 归位。
///
/// 返那几枚门闩（按需求单的格子），失败给退场码。
fn boot() -> Result<[PieToken; needs::PLIC.len()], usize> {
    // 1. 会话：本域那一侧的孔交给"生我者"——就是建本域的那枚线程（父域的装配者）。
    let sire = utask::sire().map_err(|_| E_SIRE)?;
    let channel = env::Name::new(RECORDS).map_err(|_| E_UP)?;
    let mut quay = Quay::open(sire);
    quay.seat(channel, QUAY_MS).map_err(|_| E_UP)?;

    // `seat` 交出去的那张名字牌，**本端这一枚槽里也有一份**（交出去的是副本）。
    // 父域读的是它那一份来归位；本端这一份得自己读掉——孔是单槽，牌留着就会把
    // 父域随后推来的 records 挡在槽外（症状：只收到 40 字节的牌）。

    // 2. 收配给：父域按同一张需求单推来记录，**按 `Slot` 归位**（不数第几条）。
    let mut buf = [0u8; CAP];
    let up = quay.find(channel).ok_or(E_UP)?;
    say(&alloc::format!(
        "plic: id={} sire={:?} peer={:?} hole={} at_peer={}",
        utask::self_id().map(|t| t.get()).unwrap_or(0),
        utask::sire().map(|t| t.get()).ok(),
        up.peer().get(),
        up.hole().get(),
        up.at_peer().get()
    ));
    let n = up.pull(&mut buf, QUAY_MS).map_err(|_| E_GRANT)?;
    say(&alloc::format!("plic: got {n}"));
    let mut got: [Option<PieToken>; needs::PLIC.len()] = [None; needs::PLIC.len()];
    pairing::unpack(&buf[..n], |slot, token| {
        if let Some(cell) = got.get_mut(slot) {
            *cell = Some(token);
        }
    });

    // 3. 四枚都要在：少一枚就不必继续（父域按同一张单子发货，缺格即装配错）。
    let mut out = [PieToken::new(0); needs::PLIC.len()];
    for (i, cell) in got.iter().enumerate() {
        out[i] = cell.ok_or(E_GRANT)?;
    }
    Ok(out)
}

/// 打一行。调试面是"服务还没起来的嘴"：本域没有会话、没有控制台，只有它。
fn say(msg: &str) {
    let _ = debug::put(msg);
}
