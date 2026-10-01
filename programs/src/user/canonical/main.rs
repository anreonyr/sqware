#![no_std]
#![no_main]

//! canonical — **控制台那一台**：把控制台当 UNIX 那一套 stdin / stdout，并在本域里扮
//! **终端那一侧的行规程**（canonical mode；**U 态**）。
//!
//! ```text
//!   1  上板报到：挂不上照旧干活（这一格只让板看得见本域的死）
//!   2  树那条路：`FIND /svc/drv/uart/{rx,tx}` ⇒ 两枚孔经会话授进本域表
//!   3  那一圈：轮转（`adapt/terminal.rs`）＋ 行规程（`core/discipline.rs`）
//!   4  `exit` 或 `^D` 收场（域退场 ⇒ 编排域收场 ⇒ 引导域退 ⇒ 停机；本机没有真正的 EOF）
//! ```
//!
//! **本文件只剩流程**：**纯功能**（行规程：ICRNL / ECHO / ECHOCTL / ERASE / KILL / EOF）在
//! `core/discipline.rs`，**适配**（找控制台 / 轮转那一圈）在 `adapt/`；判据与三条照实记在
//! `user/canonical/mod.rs`。行为一句话：**本域就是一台终端**——ECHO 把敲的字显出来、行规程做行
//! 编辑，交付的行没有下游（不写出去）。

extern crate alloc;
extern crate programs;

/// 适配（壳）：找控制台 / 轮转那一圈——由 bin 自己 `mod`。
mod adapt;

/// 纯功能：行规程（字节流 → 终端认的行）。
mod core;

use crate::adapt::{E_NO_CONSOLE, MS};
use env::Wait;
use protocol::communication::session::Session;
use protocol::system::operator::client as operator;
use protocol::system::operator::client::Face;
use runtime::env::unit as utask;

/// 本 bin 的 `main`：**返回类型就是它的退出账**——本域只有一种失败，故直接用 `Reason`。
#[programs::entry]
fn main() -> Result<(), env::Reason> {
    let sire = utask::sire();
    // **照实记（"上板"那一格退场：撤板那一刀）**：这一格从前开一条 `board::BERTH` 会话并报到
    // ——板据此看出它死了。板那一族的死信号整片退场（监督那一趟改读内核那一格）⇒ 这一格退场。

    // 2：树那条路：本域只开一条会话（`Session::open`）——找控制台要它。
    //
    // **照实记（这一处为什么包成 `Face`，task-2 那一刀）**：会话装好之后本域**只要**树上那几手
    // （名字 → 号 → 入口），那条线本身再不露面 ⇒ 按"已持 `Session` 则用 `Face`"把它交给
    // [`Face::of`]（它吃所有权），此后 [`adapt::console::find`] 只认一面。这正是"四面不出
    // `Face`"要的形状：调用方拿到的不是会话，是一面。
    let session =
        Session::open(sire, operator::BERTH, Wait::AtMost(MS)).map_err(|_| E_NO_CONSOLE)?;

    let console = adapt::console::find(&Face::of(session), Wait::AtMost(MS)).ok_or(E_NO_CONSOLE)?;

    // 3/4：行规程那一圈，直到收场词 / EOF。
    adapt::terminal::run(&console);
    Ok(())
}
