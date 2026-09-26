#![no_std]
#![no_main]

//! echo — **调试回显**：把**控制台服务**读到的一行原样写回去（**U 态**，见 `user/echo/mod.rs`）。
//!
//! ```text
//!   1  上板报到：`REGISTER "echo"`（板因此看得见本域的死；挂不上照旧回显）
//!   2  树那条路：开会话 + 另铸一枚问话孔给持树者
//!   3  先找控制台：`FIND /device/uart` ⇒ 那枚孔经会话授进本域表（找不到就有界重问）
//!   4  上树一趟：`PART / LAND / FIND / NAME / TRIM`（本域是第一位真客人）
//!   5  上树第二趟：**一串**——`LIST` 列根、`NAME` 按号翻名、`LIST /device`、`NAME` 一枚没铸过的号
//!   6  回显：**一条消息 = 一次排空**（字节流，边界无意义）⇒ 攒够一行写一行；读到 `exit` 退场
//!            （域退场 ⇒ 编排域收场 ⇒ 引导域退 ⇒ 停机）
//! ```
//!
//! **本文件只剩流程**：上板在 `adapt/board.rs`，找控制在 `adapt/console.rs`，上树两趟在
//! `adapt/tree.rs`，回显那一圈在 `adapt/echo.rs`；"字节流 → 终端认的行"那三条语义规则在
//! `core/line.rs`。判据与照实记（上板与登记是两件事、为什么按行、次序那件事、两条红线、
//! 为什么它也上板）在 `user/echo/mod.rs`。

extern crate alloc;
extern crate programs;

/// 适配（壳）：上板 / 找控制台 / 上树两趟 / 回显——由 bin 自己 `mod`。
mod adapt;

/// 纯功能：字节流 → 终端认的行。
mod core;

use crate::adapt::{E_NO_CONSOLE, MS};
use env::Wait;
use protocol::debug;
use protocol::system::operator as ocall;
use protocol::system::operator::client as operator;
use runtime::env::unit as utask;

/// 本域开口那一声（第一行读数）。
const READY: &str = "echo: ready";

/// 本 bin 的 `main`：**返回类型就是它的退出账**——本域只有一种失败，故直接用 `Reason`
/// （不立 `Fail` 枚举：一格不值得一个类型）。出口那一手在 [`macro@programs::entry`]，全仓一处。
#[programs::entry]
fn main() -> Result<(), env::Reason> {
    debug!("{}", READY);
    // 1：上板——**注册在回显之前**（板要能看见本域）。挂不上照旧回显。
    let reg = adapt::board::register();
    debug!("echo: reg={reg}");

    let sire = utask::sire();
    // 2：树那条路：本域只开一条会话——**先找控制台，再落自己那块牌子**（次序见 `user/echo/mod.rs`）。
    let Ok((tree, host)) = operator::open(sire, Wait::AtMost(MS)) else {
        return Err(E_NO_CONSOLE);
    };
    let Ok(talk) = operator::ask_hole(host) else {
        return Err(E_NO_CONSOLE);
    };

    // 3：**先找控制台**：`FIND /device/uart` ⇒ 那枚孔经会话授进本域表里。
    let console = adapt::console::find(&tree, talk);
    debug!("echo: console={}", console.is_some());

    // 4：上树一趟：**本域是第一位真客人**——把入口挂到树上、再查回来取一枚、剪掉一块空 Pane。
    let op = adapt::tree::trip(&tree, talk, host);
    debug!("echo: op={op}");

    // 5：上树第二趟：**一串**（列号 → 按号翻名 → 列 `/device` → 问一枚没铸过的号）。
    let seq = adapt::tree::serial(&tree, talk);
    debug!("echo: seq={seq}");

    // **返回值那一格判在消耗它的这一层**：`serial` 内部看不见自己那一趟被改坏。

    assert_eq!(seq, ocall::OK);

    let Some(console) = console else {
        return Err(E_NO_CONSOLE);
    };
    // 6：回显，直到收场词到、或那枚孔读不动了。
    adapt::echo::run(&console);
    Ok(())
}
