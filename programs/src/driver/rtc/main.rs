#![no_std]
#![no_main]

//! rtc — **第二台设备驱动**：`rtc@101000` 的持有者（设备树里那条 11 号线）。
//!
//! 它存在的理由是一条**判据**，不是一个功能：线那四格、配给、门牌、设备面这一整套，今天只有
//! `uart` 一台真设备走过——**抽象等第二个实例**。本域是第二个实例，且它管的是一台**自己会拉线**
//! 的设备（闹钟到点 ⇒ 电平起来）。
//!
//! ```text
//!   1  领配给：`rtc@101000` 那页寄存器（`ONLY`）
//!   2  开图 + **自证**：读两次它的纳秒计数器（两次不同 ⇒ 它真的在走）
//!   3  上板（板看得见本域的死）+ 上树（用来找线路由者）
//!   4  占线：报设备名（**线 = 名字的函数**），收一格答码 —— 11 号线归本域
//!   5  武装第一次闹钟（`now + PERIOD`）：到点设备拉线 ⇒ 路由者 claim ⇒ 投递到本域
//!   6  常驻：一次投递 = 一次闹钟 —— 清掉设备那一格（电平源，不清线就一直挂着）⇒ 再武装
//!      下一次 ⇒ 说一句"排空了"（路由者据此把线放回）
//! ```
//!
//! # 照实记：三处想当然被读数打回来
//!
//! 这一台在这个仓里以前没人量过，故本域起手先报一行自证读数（那对纳秒格子读两次），
//! 再靠一行行读数把设备语义坐实。第一版写了三处**想当然**，全被打回来：
//!
//! 1. **读时间**：先读高再读低（还自以为要"连读两次高、不变才算一对"）。真语义是**低半格那次
//!    读把高半格锁存起来** ⇒ 正确读法是**先低后高**，那一对天生自洽。
//! 2. **报警状态**：以为 `ALARM_STATUS` 是"到点了"，而它**在到点那一瞬是 0**（它是
//!    `alarm_running`：响过就清）。故本域改用 `IRQ_ENABLED`（闸门）与两头的时间读数说话。
//! 3. **写闹钟**：先写低半格再写高半格；真语义是**低半格那次写会当场比较一次**——首次写时高
//!    半格还是 0 ⇒ 当场判成"到点了"（实测：第一次武装在目标之前约 99 ms 就报了一次）。
//!    改成**先高后低**。
//!
//! 另量到一条：**这一格是电平源**——把 `CLEAR_INTERRUPT` 那一手临时去掉，同一段运行里投递
//! 从 5 次变成 **3093** 次（线一直挂着）。故"清掉那一格"不是客气。
//!
//! # 为什么没有服务面
//!
//! "报时 / 定闹钟"那套是**本驱动自己的具体协议**（协议层不放服务面：旧 `uart` 协议的死因就是
//! 把它放了进去）。今天没有客人要它，故本域**不落门牌**——它是装配单里的一员（`board` 让板
//! 看得见它的死、`operator` 只用来按名找线路由者），但不占树上一格。
//!
//! # 特权级
//!
//! **U 态**（`kernel/build.rs::INITRD_BINS`）：读那页寄存器、`claim` / `complete`、持门闩都不
//! 需要 S 态——驱动那一档是量出来的（见 `programs/src/driver/uart/main.rs` 头注）。

extern crate alloc;
extern crate programs;

// 共享件住驱动这一族里：`assemble` 是各驱动都要写一遍的那段客侧装配，需求单同一份源码编一次。
use programs::driver::assemble;
use programs::driver::rtc::needs;

// 板：本域是**客侧**（只装板路）；树：也是客侧（按名找线路由者）。
use protocol::operator::call as ocall;
use protocol::operator::client as operator;
use protocol::system::board::client as board;

use alloc::format;

use env::{Name, PieToken, TaskId};
use protocol::driver::line;
use protocol::session::Quay;
use runtime::core::dock::Dock;
use runtime::env::debug;
use runtime::env::mail::PolePie;
use runtime::env::room::exit_with;
use runtime::env::unit as utask;

/// 设备面（本域私有：谁的设备谁自己带）。
mod rtc;

/// 要找的那位服务（线路由者）在树上的名字。
const SERVICE: &str = "router";

/// 等板 / 等树 / 办一趟登记的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 闹钟周期（纳秒）：**每次排空之后再武装一次**，故这一台一直走着。
///
/// 100 ms（10 Hz）是有意的：短跑里也要看得见几行 `rtc: rang`（一次性闹钟在只有一两秒的验收
/// 运行里可能一行都出不来），而 10 Hz 对这台机器可以略去不计。
const PERIOD_NS: u64 = 100_000_000;

/// 本地失败编号（装配那三步用 [`assemble`] 的家族编号 1–3）。
const E_OPEN: usize = 4;
const E_BOARD: usize = 5;
const E_LINE: usize = 6;
const E_TREE: usize = 7;

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 1. 领配给：那一页寄存器（`ONLY`：同一时刻只该有一个持有者）。
    let mut slots = [None; needs::WANTS.len()];
    let got = match assemble::receive(&mut slots, needs::slot_of) {
        Ok(n) => n,
        Err(code) => exit_with(code),
    };
    let [Some(rtc_token)] = slots else {
        exit_with(assemble::E_GRANT)
    };
    say(&alloc::format!("rtc: got {got}"));

    // 2. 开图 + 自证：那对纳秒格子读两次（两次不同 ⇒ 它是活的）。
    let Ok(dock) = Dock::open(PolePie::from_token(rtc_token)) else {
        exit_with(E_OPEN)
    };
    let view = dock.view();
    let (t0, t1) = (rtc::now(view), rtc::now(view));
    say(&alloc::format!("rtc: time {t0} -> {t1}"));

    // 3. 上板（只为让板看得见本域的死）+ 上树（用来找线路由者）；一个域只开一条会话。
    let Ok(sire) = utask::sire() else {
        exit_with(E_BOARD)
    };
    let Ok((_link, board_link)) = board::open(sire, MS) else {
        exit_with(E_BOARD)
    };
    if board::ask_hole(board_link).is_err() {
        exit_with(E_BOARD);
    }
    let Ok((link, host)) = operator::open(sire, MS) else {
        exit_with(E_TREE)
    };
    let Ok(talk) = operator::ask_hole(host) else {
        exit_with(E_TREE)
    };

    // 4. 占线：报设备名（线号由路由者解树解出来，本域从不说它）。
    let Ok(held) = register(&link, talk, host) else {
        exit_with(E_LINE)
    };
    say("rtc: line occupied");

    // 5. 武装第一次闹钟：到点设备拉线 ⇒ 路由者 claim ⇒ 投递到本域。
    //    **读数带两格**：`ier=` 闸门开着没有、`alarm=` 闹钟武装着没有（`ALARM_STATUS` 是
    //    `alarm_running`，不是"到点了"——见 `rtc.rs`）。
    let mut at = rtc::now(view) + PERIOD_NS;
    rtc::arm(view, at);
    say(&alloc::format!(
        "rtc: armed at={at} ier={} alarm={}",
        rtc::irq_enabled(view),
        rtc::armed(view)
    ));

    // 6. 常驻：一次投递 = 一次闹钟。顺序与 `uart` 同一条道理——**先把设备那一格清干净**
    //    （清 `irq_pending`：电平源，不清线就一直挂着），再武装下一次，最后说"排空了"。
    let mut n = 0usize;
    loop {
        if held.receive(usize::MAX).is_err() {
            exit_with(E_BOARD);
        }
        let now = rtc::now(view);
        rtc::clear(view);
        let next = now + PERIOD_NS;
        rtc::arm(view, next);
        let _ = held.exhaust();
        n += 1;
        say(&alloc::format!("rtc: rang n={n} now={now} at={at}"));
        at = next;
    }
}

/// 从树上找到线路由者，把本域那条线登记下来。
///
/// 会话是**上面那一条**（同一个域只开一条，见 `driver/uart` 头注）；设备名取自**本域那张需求单**
/// （名字只有一处）；入口经会话从树上授进来，泊位由 `line` 那一层装。
fn register(link: &Quay, talk: PieToken, host: TaskId) -> Result<line::client::Line, ()> {
    let dir = Name::new(protocol::driver::DIR).map_err(|_| ())?;
    let want = Name::new(SERVICE).map_err(|_| ())?;
    let path = [dir, want];
    let code =
        operator::ask(talk, link, host, ocall::FIND, &path, PieToken::NONE, MS).map_err(|_| ())?;
    if code != ocall::OK {
        return Err(());
    }
    let entry = operator::take(link, host).ok_or(())?;
    let device = needs::WANTS[0].name().ok_or(())?;
    line::client::Line::occupy(entry, device, MS).map_err(|_| ())
}

/// 打一行。调试面是"服务还没起来的嘴"：本域没有控制台，只有它。
fn say(msg: &str) {
    let _ = debug::put(msg);
}
