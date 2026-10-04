#![no_std]
#![no_main]

//! 等待者（U 态）：把台主 accord 进来的那枚孔挂进那只共享组，
//! 等组键；醒来自己复核（孔槽里还有没有货），把结果推回给台主。
//! # 为什么要有它
//! 共享组的**整链放行**（≥ 2 个等待者）在树里没有别的用法：那时两个独享者各独享一只
//! **独占**组，链长恒 ≤ 1 ⇒ "放行全链"与"放行一人"在那些路径上**是同一条**。本台子造出
//! # 本端的表由台主 accord 出来（三枚），**顺序不认、按记号认**
//! 记号（`Reserve`）只长在孔上，故"认不出记号的那一枚"就是组：
//! # 读数（调试面；台主只读回报孔，不读这些行）
//! 回报孔里那个字节：`H` = 已挂、`T` = 看见货、`E` = 出错。**台子不用 `T` 表示"我拿到了"**
//! ——消息由台主取（见下）：醒来的人只用**非破坏性**的 `peek` 报"我被放行了"，
//! 否则先到的人会把判据本身吃掉（见 `group.rs` 头注）。

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Reason;

use env::{HoleDir, Mark, PieToken};
use protocol::debug;
use runtime::core::res::pile::Pile;
use runtime::core::res::pie::{HolePie, TolePie, pies};

#[programs::entry]
fn main() -> Reason {
    let (Some(group), Some(member), Some(report)) = discover() else {
        return bail("waiter: table incomplete");
    };

    let pile = Pile::new(TolePie::from_token(group));
    let member = HolePie::from_token(member);
    let report = HolePie::from_token(report);

    // 挂一格：**一个方向就够**（`Pull` = "有东西可读"）。
    if pile.attach(&member, HoleDir::Pull).is_err() {
        return bail("waiter: attach");
    }
    // 先报"已挂"：台主收齐两枚才投信 ⇒ 投信那一刻两人**都在等**（判据成立的前提）。
    if report.push(b"H", Wait::Forever).is_err() {
        return bail("waiter: report");
    }
    if !matches!(report.wait(HoleDir::Push, Wait::Forever), Ok(true)) {
        return bail("waiter: report");
    }
    debug!("waiter: hung");

    // ★ 正路：共享组上**多个等待者挂在同一只组键上**。
    // 手上没东西 ⇒ 那一次只是"快照变了"的提示 ⇒ **继续等**（契约：`await_` 的返回只是提示，
    // 别把一次返回当终局）。
    loop {
        if pile.await_(Wait::Forever).is_err() {
            return bail("waiter: await");
        }
        match member.peek() {
            Ok(_) => {
                debug!("waiter: woke");
                // **递出 ＋ 等它被取走**：字节是这一帧的局部，不等它下线就返回，台主可能复制到死栈。
                let word = [b'T'];
                let _ = report.push(&word, Wait::Forever);
                let _ = report.wait(HoleDir::Push, Wait::Forever);
                break;
            }
            Err(e) if e.source.is_busy() => continue,
            Err(e) => {
                debug!("waiter: err={}", e.source.code());
                let word = [b'E'];
                let _ = report.push(&word, Wait::Forever);
                let _ = report.wait(HoleDir::Push, Wait::Forever);
                break;
            }
        }
    }
    return 0;
}

/// 认领本端那三枚：记号认孔，剩下那一枚是组
/// 或它已封印）——内核在 `Collect` 里先问死活、再问是不是孔，两类情形落同一格
fn discover() -> (Option<PieToken>, Option<PieToken>, Option<PieToken>) {
    let (mut group, mut member, mut report) = (None, None, None);
    for p in pies() {
        if p.mark == Mark::of("member") {
            member = Some(p.token);
        } else if p.mark == Mark::of("report") {
            report = Some(p.token);
        } else if p.mark == Mark::NONE {
            // 记号只长在孔上 ⇒ 认不出记号的就是组。
            group = Some(p.token);
        }
    }
    (group, member, report)
}

/// 起不来就报哪一句（内核收场时把这一句连同域号打出来）
fn bail(msg: &str) -> Reason {
    debug!("{}", msg);
    1
}
