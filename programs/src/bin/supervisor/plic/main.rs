#![no_std]
#![no_main]

//! plic — **中断面域**：外部中断的收与结（S 态 supervisor 域，一个线程）。
//!
//! ```text
//! 收配给（父域按需求单推来记录）
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
//! 父域知道本域的句柄，故"父 → 子"这一向不用协议。难的是反向：**子域拿到一枚门闩时不知道
//! 它是哪一枚**（`collect` 只报句柄、权限位、谁授的，不报类型与名字）。故本域：
//!
//! ```text
//! 1  自铸一条控制孔 → 授给父域（此刻父域在自己表里能按 vestor == 本域 认出它）
//! 2  父域按需求单把要的几样 Accord 给本域，再把「名字 + 句柄」的记录推进那条孔
//! 3  本域 pull 出记录，按名字认领
//! ```
//!
//! 记录用的是 `env::Pair`——**内核交给父域的就是这个格式**（配对块），此处不新造。
//!
//! # 第一刀：还没有客户端
//!
//! 本域只做"收与结"：接上所有线、claim、complete、应铃。**没有**"名字 → 线号"的表，
//! 也没有投递给客户端那一段——那是第二刀。
//!
//! 本域**不读走设备里的字节**：`serial@10000000` 的接收字节归 console（今天归内核的调试
//! 面），读走就是把用户的输入从它手里抢走。不读 ⇒ 源头一直挂着电平，故每领一条线就把它
//! **静音**（`priority = 0`），等铃静下来一拍再把线放回去。这不只是权宜：**按线静音**本来
//! 就是驱动域自己的细杠杆，第一刀正好把它走一遍。

extern crate alloc;
// 本包 lib 提供 `_start` + panic_handler；必须真的链接它，`use` 只带符号不算。
extern crate programs;

/// 设备侧（本域私有，同 `root/manifest.rs` 的纪律：谁的设备谁自己带）。
mod plic;
mod uart;

use crate::plic::{LINE_PRIORITY, Plic};
use env::{PAIR_LEN, Pair};
use programs::needs::{self, Slot};
use runtime::core::bell::Bell;
use runtime::core::dock::Dock;
use runtime::core::port::{Access, Policy, ship};
use runtime::env::debug;
use runtime::env::mail::{HolePie, NolePie, PolePie};
use runtime::env::room::exit_with;
use runtime::env::unit as utask;

/// 域自己的正常退场码不作数（本域是常驻的，直到父域退场时被级联扑杀）。
///
/// 失败编号指"死在装配的哪一步"。
const E_SIRE: usize = 1;
const E_UP: usize = 2;
const E_SHIP: usize = 3;
const E_GRANT: usize = 4;
const E_OPEN: usize = 5;
const E_TREE: usize = 6;
const E_BELL: usize = 7;

/// 等配给的上界（毫秒）。父域此刻正在 `ship`，故这个值只需要够它跑完几步。
const GRANT_MS: usize = 1000;

/// 有静音的线时等铃的上界（毫秒）。**只有这时才用节拍**——闲的时候本域真睡（见主循环）。
const IRQ_WAIT_MS: usize = 20;

/// 静音位图能记到第几条线（`u64` 一位一条）。
///
/// 越界的线不记账：当场放回即可（virt 上 `ndev` 是 32，这条够不着）。
const MUTE_BITS: u32 = 64;

/// 收记录用的缓冲：需求单几条就备几条（父域按**同一张单子**推）。
const REC_BYTES: usize = PAIR_LEN * needs::PLIC.len();

/// 日志节流：前几次全打（那是"通了没有"的读数），之后每这么多条打一次。
const LOG_FIRST: usize = 16;
const LOG_EVERY: usize = 64;

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 1. 控制孔：本域收配给的入口。授给父域（它往这里推"这三样在哪"）。
    let Ok(sire) = utask::sire() else {
        exit_with(E_SIRE);
    };
    let Ok(up) = HolePie::unseal() else {
        exit_with(E_UP);
    };
    if ship(&up, sire, Access::READ | Access::WRITE, Policy::NONE).is_err() {
        exit_with(E_SHIP);
    }

    // 2. 收配给：记录一条一枚，名字用的是**与父域同一张需求单**（`programs::needs::PLIC`）
    //    ——两端不各抄一份名字，归位也按单子上的 `Slot`，不数第几条。
    let mut buf = [0u8; REC_BYTES];
    let Ok(n) = up.pull_timeout(&mut buf, GRANT_MS) else {
        exit_with(E_GRANT);
    };
    let mut got: [Option<usize>; needs::PLIC.len()] = [None; needs::PLIC.len()];
    for i in 0..n / PAIR_LEN {
        // SAFETY: 记录与块同源（`Pair` 的尺寸由编译期断言锁死为 `PAIR_LEN`）；缓冲只保证
        // 1 字节对齐，故用 `read_unaligned`。越界的那部分由 `n / PAIR_LEN` 挡掉。
        let rec =
            unsafe { core::ptr::read_unaligned(buf.as_ptr().add(i * PAIR_LEN).cast::<Pair>()) };
        let Some(name) = rec.name() else { continue };
        for need in needs::PLIC {
            if need.name == name.as_str() {
                got[need.slot as usize] = Some(rec.token().get());
            }
        }
    }
    let (Some(plic_token), Some(dtb_token), Some(irq_token), Some(src_token)) = (
        got[Slot::Plic as usize],
        got[Slot::Dtb as usize],
        got[Slot::Bell as usize],
        got[Slot::Source as usize],
    ) else {
        exit_with(E_GRANT);
    };

    // 3. 开图 + 读树：控制器、本域的 context、要接的线。
    let Ok(plic_dock) = Dock::open(PolePie::from_token(plic_token)) else {
        exit_with(E_OPEN);
    };
    let Ok(dtb_dock) = Dock::open(PolePie::from_token(dtb_token)) else {
        exit_with(E_OPEN);
    };
    let Ok(src_dock) = Dock::open(PolePie::from_token(src_token)) else {
        exit_with(E_OPEN);
    };
    let Some((plic, lines)) = Plic::new(plic_dock.view(), dtb_dock.view()) else {
        exit_with(E_TREE);
    };
    let bell = Bell::new(NolePie::from_token(irq_token));

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

    // 4. 循环：等铃 → 领到空 → 逐条静音 + 结 → 应铃 → 到点把静音的放回去。
    //
    // **闲的时候真睡**：`muted == 0` 时 wait 传 `usize::MAX`（永久挂起），本域一次都不醒。
    // 只有手里还压着静音的线时才退回节拍——那是第一刀唯一能"放回"的手段（没有客户端会来
    // 说一声"我抽干了"，见下面第 5 条）。第二刀把放回换成事件，这个节拍随之消失。
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
            // 本域回到永久挂起。原先这里是把所有线无条件重写一遍：闲着也每 20 ms 写 10 条。
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

/// 打一行。调试面是"服务还没起来的嘴"：本域没有会话、没有控制台，只有它。
fn say(msg: &str) {
    let _ = debug::put(msg);
}
