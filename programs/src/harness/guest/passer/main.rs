#![no_std]
#![no_main]

//! 起来、挂一个名字、直接死（不说再见）。
//! 它是板上那两本账的"死"判据（**"那一枚入口还答得出吗"**，`VestedBy` = `Reserve` 那一格）的
//! **读数程序**：与 `guest` 的区别只有一处——**它不说 `EVICT`**。故板上那枚牌子与板侧那一格
//! 本程序的全部价值就在**少说那一句**：它一死，它开的那些门随之封印（退出钩子），板侧那
//! 一格与板上那枚牌子都成了死实例，而 `Reserve` 对已封印的门答 `Err` ⇒ **板问得出来**。
//! # 特权级由清单定
//! 都不需要 S 态。

extern crate alloc;
extern crate programs;

use programs::Report;

const ME: &str = "passer";

const MS: usize = 1000;

const E_OK: usize = 0;
const E_TRIP: usize = 1;

#[programs::entry]
fn main() -> Report<'static> {
    return Report::note(E_OK, "passer: gone");
}

/// 哪里算不下去就报哪一句（kernel 收场时把这一句连同域号打出来）
fn bail<'a>(note: &'a str) -> Report<'a> {
    return Report::note(E_TRIP, note);
}
