#![no_std]
#![no_main]

//! plic — **中断面域**：外部中断的收与结（S 态 supervisor 域，**两枚线程**）。
//!
//! ```text
//! 装会话（交一枚孔给父域，父域按名字认领）
//!   → 收配给（父域按同一张需求单推来记录，按 Slot 归位）
//!   → 板上一趟（挂上本域的服务入口、查回来验一遍：答话三格 + 入口有没有到手）
//!   → 接上控制器自报的每一条线
//!   → 两枚线程各守一个源：
//!       主线程     等铃 → claim 到空 → 按线静音 + complete → 应铃 → 到点把线放回去
//!       待客线程   守服务入口（有人敲就记一行、把本域的名字答回去）
//! ```
//!
//! # 为什么是两枚线程
//!
//! 本域有两件事要等：**铃**（外部中断）与**服务入口**（有人按名字找上门）。今天没有"同时
//! 等两个源"——门铃与孔各有各的唤醒键，合并不了；塞进同一枚线程就得靠节拍轮询，而"闲时
//! 真睡"那条读数会跟着丢掉。**一枚线程守一个源**是今天唯一不退化的办法。
//!
//! # 服务入口为什么由待客线程铸
//!
//! `PieToken` 是"**我这张表**里的第几个"——**同一个域里两枚线程各有一张表，号过不了线**。
//! 客人推上来的那一句得由**守在那扇门上**的那枚线程读走，故入口由它铸、它读；本域主线程
//! 要的是**副本**（拿去挂到板上），那一枚经 `Ship` 过来（同域转授，与跨域同一条路）。
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
//! # 还没有的那一格：投递给客户端
//!
//! 本域做"收与结"：接上所有线、claim、complete、应铃。**没有**"名字 → 线号"的表，也没有
//! 把中断投递给客户端那一段——今天来敲门的只有 `guest`（按名字问一句、本域答一句），
//! 还没有谁是"要这条线的中断"的人。
//!
//! 本域**不读走设备里的字节**：`serial@10000000` 的接收字节归 console。不读 ⇒ 源头一直
//! 挂着电平，故每领一条线就把它**静音**（`priority = 0`），等铃静下来一拍再把线放回去。
//! 按线静音本来就是驱动域自己的细杠杆。

extern crate alloc;
// 本包 lib 提供 `_start` + panic_handler；必须真的链接它，`use` 只带符号不算。
extern crate programs;

// 共享物住在 supervisor 目录里，由两个 bin 各自声明一次（见 `needs.rs` 头注）。
#[path = "../board.rs"]
// 本域只用**客侧**那三手（板侧那一半归 root）⇒ 另一半在这里是死码。
#[allow(dead_code)]
mod board;
#[path = "../needs.rs"]
mod needs;
#[path = "../pairing.rs"]
mod pairing;

/// 设备侧（本域私有，同 `lib.rs` 的纪律：谁的设备谁自己带）。
mod plic;
mod uart;

use env::PieToken;
use env::wire::PAIR_LEN;
use protocol::board::call as bcall;
use protocol::session::Quay;
use runtime::core::bell::Bell;
use runtime::core::dock::Dock;
use runtime::core::port::{self, Access, Policy};
use runtime::core::unit::{Join, closure};
use runtime::env::debug;
use runtime::env::mail;
use runtime::env::mail::{NolePie, PolePie};
use runtime::env::room::exit_with;
use runtime::env::unit as utask;

use crate::plic::{LINE_PRIORITY, Plic};

/// 与父域之间那条通道的名字（两端按它对位，不靠位置约定）。
const RECORDS: &str = "records";

/// 本域挂在板上的名字，以及**对照**用的那个"板上没有的名字"。
///
/// 对照那一问是必要的：只报"查到了"而不知道"查不到会怎样"，那一格读数证明不了什么。
const SERVICE: &str = "plic";
const NOBODY: &str = "no-such-name";

/// 与待客线程之间那条路的名字（本域登记它交回来的服务入口时用；它那侧不看名字）。
const DESK_LINK: &str = "plic-entry";

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
const E_DESK: usize = 7;

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
    // 只有手里还压着静音的线时才退回节拍——那是今天唯一能"放回"的手段（还没有谁会来说
    // 一声"我抽干了"）。把放回换成事件，这个节拍随之消失。
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

/// 装配的前半：装会话 → 收配给 → 按 `Slot` 归位 → 板上一趟。
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

    // 4. 板那条路：**趁本端表里还是干净的**先装上——`pair` 认的是"我没开过的那一枚"，
    //    而后面那一步（待客线程把入口副本交回来）也会往本端表里放一枚外来孔。
    let link = board::open(sire, QUAY_MS).ok();

    // 5. 待客线程 + 服务入口：入口由待客线程铸（谁守那扇门谁读它），副本本域登记。
    //    **它不拦装配**：起不来就报一句读数（板是"起来之后"的事，不是起来的条件）。
    let Ok(me) = utask::self_id() else {
        return Err(E_GRANT);
    };
    let entry = start_desk(me);

    // 6. 板上一趟：挂上这一枚入口、再查回来验一遍。
    let trip = match (link, entry) {
        (Some(link), Some(entry)) => board_trip(&link, entry),
        _ => None,
    };
    match &trip {
        Some(t) => say(&alloc::format!(
            "plic: board reg={} miss={} hit={} entry={} grant={}",
            t.reg,
            t.miss,
            t.hit,
            t.entry.get(),
            t.grant.map(|g| g.get()).unwrap_or(0)
        )),
        None => say("plic: board: no link"),
    }
    Ok(out)
}

/// 起待客线程，并把**它的服务入口**收回来（本域登记用的那一枚副本）。
///
/// 入口由待客线程铸（`PieToken` 过不了线，见文件头），副本经 `Ship` 交到本线程表里；
/// 本线程另开一座码头认它——判据是 `owner == 待客线程`（铸的人就是 owner）。
///
/// `seat` 那一步也会把本线程铸的一枚交给它（本协议里"认领"要求本端先装一条）：它不用，
/// 也不碍事。
fn start_desk(me: env::TaskId) -> Option<PieToken> {
    let node: Join<()> = closure(move || desk(me));
    let id = node.id();
    drop(node);
    let link = env::Name::new(DESK_LINK).ok()?;
    let mut quay = Quay::open(id);
    quay.seat(link, QUAY_MS).ok()?;
    quay.claim(id, QUAY_MS).ok()?;
    let pier = quay.find(link)?;
    Some(PieToken::new(pier.at_peer().get()))
}

/// 待客：服务入口上有人说话，就记一行、把本域的名字答回去。
///
/// **一枚线程守一个源**（见文件头）：本线程只等这一枚孔，故 `usize::MAX` = 真挂起。
/// 一问一答各 32 字节（一个名字，与牌子同一个解码面），答话走**同一枚孔**——单槽，
/// 一问一答交替（对面推、本端取、本端推、对面取）。
fn desk(me: env::TaskId) -> ! {
    // 服务入口：**本线程铸、本线程读**（`PieToken` 过不了线，见文件头）。
    let Ok(entry) = mail::unseal_hole() else {
        exit_with(E_DESK)
    };
    // 副本交给本域主线程：它拿去挂到板上——别人按名字找到的就是这一扇门。
    let said = port::ship(
        &mail::HolePie::from_token(entry),
        me,
        Access::READ | Access::WRITE,
        Policy::VEST,
    );
    if said.is_err() {
        exit_with(E_DESK)
    }
    let hole = mail::HolePie::from_token(entry);
    let Ok(our) = env::Name::new(SERVICE) else {
        exit_with(E_DESK)
    };
    loop {
        let mut buf = [0u8; env::wire::NAME_LEN];
        // **一并取回发送者**：答话要推到"这位客人借给我的那一枚回信孔"上，而"是谁"
        // 由内核在推的那一刻盖章（报文里没有来源字段，也不必有）。
        let Ok((n, from)) = hole.pull_timeout_from(&mut buf, usize::MAX) else {
            exit_with(E_DESK)
        };
        // 长度不对 = 对面送来的不是一个名字：照说一句、照答一句（不猜内容）。
        let who = (n == env::wire::NAME_LEN)
            .then(|| env::Name::from_bytes(buf).ok())
            .flatten();
        let who = who.as_ref().map(env::Name::as_str).unwrap_or("?");
        say(&alloc::format!("plic: desk {who}"));
        // 回信孔：本端表里 `owner` 是这位客人的那一枚（副本共享 owner、转手不变）。
        let Some(back) = opened_for(from) else {
            continue;
        };
        if mail::HolePie::from_token(back).push(our.bytes()).is_err() {
            exit_with(E_DESK)
        }
    }
}

/// 本端表里**这位的那一枚孔**（客人借过来的回信孔）：`owner == who`，取最后登记的那一枚。
///
/// 判据落在 `owner` 上（副本共享同一事实、转手不变）：本端自己铸的每一枚 owner 都是本端，
/// 客人交进来的那一枚 owner 是客人——**编号比不出来，这一格比得出来**。
fn opened_for(who: env::TaskId) -> Option<usize> {
    let mut index = 0usize;
    let mut found = None;
    loop {
        let (token, _perm, _vestor) = mail::collect(index).ok()?;
        // 越界哨兵：这一遍扫完了。
        if token.get() == 0 {
            return found;
        }
        index += 1;
        if mail::reserve(token).ok().map(|(_v, owner)| owner) == Some(who) {
            found = Some(token.get());
        }
    }
}

/// 板上一趟的读数（`None` = 路都没装上）。
///
/// ```text
///   entry  本域的服务入口（自己铸的那一枚）          —— 谁想跟本域说话就往它推
///   reg    REGISTER "plic" 的答话码                  —— 0 = 板收下了
///   miss   LOOKUP   "no-such-name" 的答话码          —— 该是 1（UNKNOWN）
///   hit    LOOKUP   "plic" 的答话码                  —— 0 = 查到并把入口授了回来
///   grant  查回来那一枚的号（**通得过自检的**才算）  —— 必须**不是** entry 本身
/// ```
///
/// 五个数各是一格，缺一格这句话就证明不了什么：没有 `miss`，"查到了"是空话；没有
/// `grant` 与 `entry` 两个号，"授回来的指回原物"是空话。
struct BoardTrip {
    entry: PieToken,
    reg: u8,
    miss: u8,
    hit: u8,
    grant: Option<PieToken>,
}

/// 板上那一趟：**挂上自己那一枚入口，再查回来验一遍**。返一格读数（`None` = 路都没装上）。
///
/// ```text
///   REGISTER "plic"          → 板上挂了本域的服务入口（入口经会话交给板）
///   LOOKUP   "no-such-name"  → 板上没有的名字必须答 UNKNOWN（否则"查到了"不值钱）
///   LOOKUP   "plic"          → 查回来一枚入口（板经会话授进本域表里）
///   自检                     → 往授进来的那一枚推一个字节，从**本域那一枚**读回来
/// ```
///
/// 第四格是**板授进来的那一枚**（客侧按 `vestor` 认，见 `board::take`）：本域查到的是不是
/// 自己那一枚，由它证明——`None` = 板答了"查到了"却没把入口交进来。
///
/// **推读自检已经拆掉**：本域的服务入口由待客线程读（两个方向各一枚孔），本线程推一句
/// 进去只会被它读走——"授进来的指回原物"这件事改由**真客人**（`guest`）走一遍，那比自问
/// 自答结实。
fn board_trip(link: &Quay, entry: PieToken) -> Option<BoardTrip> {
    let name = env::Name::new(SERVICE).ok()?;
    let absent = env::Name::new(NOBODY).ok()?;
    let none = PieToken::new(0);
    let reg = board::ask(link, bcall::REGISTER, name, entry, QUAY_MS).ok()?;
    let miss = board::ask(link, bcall::LOOKUP, absent, none, QUAY_MS).ok()?;
    let hit = board::ask(link, bcall::LOOKUP, name, none, QUAY_MS).ok()?;
    let grant = board::take(link);
    Some(BoardTrip {
        entry,
        reg,
        miss,
        hit,
        grant,
    })
}

/// 打一行。调试面是"服务还没起来的嘴"：本域没有会话、没有控制台，只有它。
fn say(msg: &str) {
    let _ = debug::put(msg);
}
