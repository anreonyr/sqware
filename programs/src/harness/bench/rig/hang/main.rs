#![no_std]
#![no_main]

//! 握手版受害者（rig A）：无限挂在自己的孔上，由台主 push 唤醒。
//! # 为什么需要它
//! 受害者一起来就自己转"1 ms 空转 / 1 ms 睡"是不行的：放行那一刻它在**本核**队列里排队，
//! 台主不阻塞时它根本不上台——台子量到的全是"在容器里被杀"的快路径。要量"点名落在它
//! **离核那一瞬**"那一格，上台/离核必须**由台主控制**。
//! 本程序把控制权交给台主：
//! **为什么不自己校准**：`tick::calibrate()` 一次要 ~0.4 s（睡 200 ms + 忙等两格刻度），
//! 而台子是**每轮一个新受害者**——自己校准等于每轮白扔 0.4 s。台主本来就要校准一次
//! 第一次唤醒**，此后每一句都只是"该上台了"。
//! 与 `churn` 一样：不铸会话以外的东西、不要门闩；特权级由清单定（U 态）。

extern crate programs;

use env::Wait;
use programs::Reason;

use programs::harness::tick;

use ipc::session::establish;
use programs::debug;
use env::unit;
use ::resource::raw::{Hole};

/// 本端那枚泊位的名字（同时刻在孔上）：台主按这个名字认领它
const MARK: &str = "wake";

/// 诊断开关：每域打一行"我被唤醒了"
/// 查"台主那一记 push 到底有没有把它唤醒"时打开；**默认关**——它会把每轮的时间轴搅浑
/// （打印要过控制台），一轮 328 次就是 328 行。实测（打开时）：328 次试验里只打出 2 行
/// 即"push 能到，但到得极少"
const REPORT_WAKE: bool = false;

#[programs::entry]
fn main() -> Reason {
    let sire = unit::sire();

    // 一手就是"两头都装"：铸本端那一枚（刻 `wake` 的记号）交给生我者——台主认领它，于是台主
    // 手里有写端、推得醒本端——并顺手试认它那一枚（`POLL` = 不等：**它本端用不上**，本端只读
    // 自己那一枚）。
    let Ok(pair) = establish::endpoint(sire, env::Mark::of(MARK), Wait::POLL) else {
        return bail("hang: seat");
    };
    let pie = Hole::from_raw(pair.rx());

    // 第一句 = "在台上跑多少轮"（前 4 字节小端）。拿不到就退化成"在台上不占时间"。
    let mut buf = [0u8; 8];
    let mut burst = 0usize;
    if let Ok((n, _)) = pie.pull(&mut buf, Wait::Forever)
        && n >= 4
    {
        burst = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        // 诊断探针（默认关）：一行一域，回答"台主那一记 push 到底有没有把它唤醒"。
        if REPORT_WAKE {
            debug!("hang: woke");
        }
    }

    loop {
        tick::spin(burst);
        // ★ 离核：无限挂在自己的孔上——Wait::Forever = 永久等，被 push 才醒。
        let _ = pie.pull(&mut buf, Wait::Forever);
    }
}

/// 起不来就报哪一句（kernel 收场时把这一句连同域号打出来）——**并把原因码交回调用方**
fn bail(msg: &str) -> Reason {
    debug!("{}", msg);
    1
}
