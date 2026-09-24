#![no_std]
#![no_main]

//! waiter — 共享组台的**等待者**（U 态）：把台主 accord 进来的那枚孔挂进那只共享组，
//! 等组键；醒来自己复核（孔槽里还有没有货），把结果推回给台主。
//!
//! # 为什么要有它
//!
//! 共享组的**整链放行**（≥ 2 个等待者）在树里没有别的用法：`root` / `board` 各独享一只
//! **独占**组，链长恒 ≤ 1 ⇒ "放行全链"与"放行一人"在那些路径上**是同一条**。本台子造出
//! 两个真正的等待者，让那条新路第一次被走到；"一次事件只归一个人"那半边也在这里量。
//!
//! # 本端的表由台主 accord 出来（三枚），**顺序不认、按记号认**
//!
//! 记号（`Reserve`）只长在孔上，故"认不出记号的那一枚"就是组：
//!
//! ```text
//!   记号 "member" ⇒ 要等的那枚孔
//!   记号 "report" ⇒ 回报孔（**一名等待者一枚**：孔是单槽，共用会撞 `Busy`）
//!   认不出记号    ⇒ 组（本端表里唯一一枚非孔）
//! ```
//!
//! # 读数（调试面；台主只读回报孔，不读这些行）
//!
//! ```text
//!   waiter: hung        ← 已挂进组并报过"已挂"，接着就去等
//!   waiter: woke        ← 醒来后 `peek` 看见槽里有货（**我被这次投信放行了**）
//!   waiter: err=<码>    ← 别的错（台子红了，码给出来）
//! ```
//!
//! 回报孔里那个字节：`H` = 已挂、`T` = 看见货、`E` = 出错。**台子不用 `T` 表示"我拿到了"**
//! ——消息由台主取（见下）：醒来的人只用**非破坏性**的 `peek` 报"我被放行了"，
//! 否则先到的人会把判据本身吃掉（见 `group.rs` 头注的照实记）。

extern crate alloc;
extern crate programs;

use programs::Reason;

use env::HoleDir;
use env::Mark;
use env::PieToken;
use runtime::core::tole::Tole;
use runtime::env::debug;
use runtime::env::mail::{self, HolePie};
use runtime::env::tole::TolePie;

#[programs::entry]
fn main() -> Reason {
    let (Some(group), Some(member), Some(report)) = discover() else { return bail("waiter: table incomplete") };

    let tole = Tole::new(TolePie::from_token(group));
    let member = HolePie::from_token(member);
    let report = HolePie::from_token(report);

    // 挂一格：**一个方向就够**（`Pull` = "有东西可读"）。
    if tole.attach(&member, HoleDir::Pull).is_err() {
        return bail("waiter: attach");
    }
    // 先报"已挂"：台主收齐两枚才投信 ⇒ 投信那一刻两人**都在等**（判据成立的前提）。
    if report.push(b"H").is_err() {
        return bail("waiter: report");
    }
    say("waiter: hung");

    // ★ 正路：共享组上**多个等待者挂在同一只组键上**。
    //
    // 醒来要能分出两件事：① **我被这次投信放行了**（这才是被测的"整链放行"）；② 槽里那条
    // 消息归谁。①用 **`peek`（看一眼、一个字节都不取）**做判据——它**非破坏性**，故两个
    // 等待者能**同时**看到"有货"；直接 `pull` 则先到的人把槽清空，第二个人醒来只看见空槽，
    // **分不出"被放行却没拿到"与"根本没被放行"**（第一版就是这么写的，实测在单播下也照样
    // 报 woke=2 —— 台子因此不自证伪；见 `group.rs` 头注的照实记）。消息由台主在收齐两份
    // 回报之后自己取走，那边顺带量"交付只归一人"。
    //
    // 槽空 ⇒ 那一次只是"快照变了"的提示 ⇒ **继续等**（契约：`await_` 的返回只是提示，
    // 别把一次返回当终局）。
    loop {
        if tole.await_(usize::MAX).is_err() {
            return bail("waiter: await");
        }
        match member.peek() {
            Ok(_) => {
                say("waiter: woke");
                let _ = report.push(b"T");
                break;
            }
            Err(e) if e.source.is_busy() => continue,
            Err(e) => {
                say(&alloc::format!("waiter: err={}", e.source.code()));
                let _ = report.push(b"E");
                break;
            }
        }
    }
    return 0;
}

/// 认领本端那三枚：记号认孔，剩下那一枚是组。
fn discover() -> (Option<PieToken>, Option<PieToken>, Option<PieToken>) {
    let (mut group, mut member, mut report) = (None, None, None);
    for i in 0.. {
        let Ok((tok, _perm, _vestor)) = mail::collect(i) else {
            break;
        };
        // 越界 ⇒ 全哨兵（`Collect` 契约：不报错）。
        if tok == PieToken::NONE {
            break;
        }
        match mail::reserve(tok) {
            Ok((_, _, mark)) if mark == Mark::of("member") => member = Some(tok),
            Ok((_, _, mark)) if mark == Mark::of("report") => report = Some(tok),
            Ok(_) => {}
            // 记号只长在孔上 ⇒ 认不出记号的就是组。
            Err(_) => group = Some(tok),
        }
    }
    (group, member, report)
}

/// 打一行。调试面是本域唯一的嘴。
fn say(msg: &str) {
    let _ = debug::put(msg);
}

/// 起不来就报哪一句（内核收场时把这一句连同域号打出来）。
fn bail(msg: &str) -> Reason {
    say(msg);
    1
}

