#![no_std]
#![no_main]

//! hang — 他杀台的**握手版受害者**（rig A）：无限挂在自己的孔上，由台主 push 唤醒。
//!
//! # 为什么需要它
//!
//! 旧版台子（`churn`）是"放行即跑"：受害者一起来就自己转"1 ms 空转 / 1 ms 睡"，而**放行那
//! 一刻它在本核队列里排队**——台主不阻塞时它根本不上台。实测 520 次试验 `now=499`（≈96% 是
//! "在容器里被杀"）⇒ 台子量到的全是**快路径**：要量"点名落在它**离核那一瞬**"那一格，
//! 上台/离核必须**由台主控制**。
//!
//! 本程序把控制权交给台主：
//!
//! ```text
//!   Quay::open(生我者) → seat("wake")      // 把自己的孔交给台主；台主认领 ⇒ 台主有写端
//!   从孔上读第一句（= 台主给的"在台上跑多少轮"，顺带就是第一次唤醒）
//!   loop { 空转那么多轮（在台上）; pull(自己那一枚, 永久)（★ 离核） }
//! ```
//!
//! **为什么不自己校准**：`tick::calibrate()` 一次要 ~0.4 s（睡 200 ms + 忙等两格刻度），
//! 而台子是**每轮一个新受害者**——自己校准等于每轮白扔 0.4 s。台主本来就要校准一次
//! （空载、铺负荷之前），故把"在台上跑多少轮"当**第一条消息**发过来；那一句**同时也是
//! 第一次唤醒**，此后每一句都只是"该上台了"。
//!
//! 与 `churn` 一样：不铸会话以外的东西、不要门闩；特权级由清单定（U 态）。

extern crate programs;

use harness::tick;

use env::Name;
use protocol::session::Quay;
use runtime::env::debug;
use runtime::env::room::exit_with;
use runtime::env::unit as utask;

/// 本端那枚泊位的名字（同时刻在孔上）：台主按这个名字认领它。
const MARK: &str = "wake";

/// 诊断开关：每域打一行"我被唤醒了"。
///
/// 查"台主那一记 push 到底有没有把它唤醒"时打开；**默认关**——它会把每轮的时间轴搅浑
/// （打印要过控制台），一轮 328 次就是 328 行。实测（打开时）：328 次试验里只打出 2 行，
/// 即"push 能到，但到得极少"。
///
/// **照实记**：这一句原先跟着"见 `rig.rs` 头注的照实记"——**rig.rs 里查不到那条记录**，
/// 那两个数是打开本开关当场量到的，没有留在树上。
const REPORT_WAKE: bool = false;

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    let Ok(sire) = utask::sire() else {
        bail("hang: no sire")
    };
    let Ok(mark) = Name::new(MARK) else {
        bail("hang: bad mark")
    };

    // 码头朝生我者：把本端那一枚孔交出去（台主认领它 ⇒ 台主手里有写端，推得醒本端）。
    let mut quay = Quay::open(sire);
    let Ok(pie) = quay.seat(mark).map(|p| *p) else {
        bail("hang: seat")
    };

    // 第一句 = "在台上跑多少轮"（前 4 字节小端）。拿不到就退化成"在台上不占时间"。
    let mut buf = [0u8; 8];
    let mut burst = 0usize;
    if let Ok(n) = pie.pull(&mut buf, usize::MAX)
        && n >= 4
    {
        burst = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        // 诊断探针（默认关）：一行一域，回答"台主那一记 push 到底有没有把它唤醒"。
        if REPORT_WAKE {
            say("hang: woke");
        }
    }

    loop {
        // ★ 在台上：跑一小段（台主扫的 `d` 就落在这一段的时序上）。
        tick::spin(burst);
        // ★ 离核：无限挂在自己的孔上——`usize::MAX` = 永久等，被 push 才醒。
        let _ = pie.pull(&mut buf, usize::MAX);
    }
}

/// 打一行。调试面是本域唯一的嘴。
fn say(msg: &str) {
    let _ = debug::put(msg);
}

/// 起不来就报哪一句（kernel 收场时把这一句连同域号打出来）。
fn bail(msg: &str) -> ! {
    say(msg);
    exit_with(1)
}
