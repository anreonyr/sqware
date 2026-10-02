#![no_std]
#![no_main]

//! canonical — **控制台那一台**：把控制台当 UNIX 那一套 stdin / stdout，并在本域里扮
//! **终端那一侧的行规程**（canonical mode；**U 态**）。
//! ```text

extern crate alloc;
extern crate programs;

/// 适配（壳）：找控制台 / 轮转那一圈——由 bin 自己 `mod`。
mod adapt;

/// 纯功能：行规程（字节流 → 终端认的行）。
mod core;

use crate::adapt::{E_NO_CONSOLE, MS};
use env::Wait;
use programs::driver::uart::client;
use protocol::communication::rack::Mode;
use protocol::communication::session::Session;
use protocol::service::operator::client as operator;
use protocol::service::operator::client::Face;
use runtime::env::unit as utask;

/// 本 bin 的 `main`：**返回类型就是它的退出账**——本域只有一种失败，故直接用 `Reason`。
#[programs::entry]
fn main() -> Result<(), env::Reason> {
    let sire = utask::sire();

    // 2：树那条路：本域只开一条会话（`Session::open`）——找控制台要它。
    let session =
        Session::open(sire, operator::BERTH, Wait::AtMost(MS)).map_err(|_| E_NO_CONSOLE)?;

    // 3：两块门牌各是一具架的页（页上那一位即铃）——客人这一面念的是**产品那一层**
    // （`driver::uart::client`），本域不自己拼名字、也不碰那一页的布局。
    // `Mode::Oldest` 是**本端（写端）**的规矩：回显推得太快时顶掉最旧未读那一格（丢有数）。
    let road = client::road().ok_or(E_NO_CONSOLE)?;
    let mut console = client::find(&Face::of(session), &road, Mode::Oldest, Wait::AtMost(MS))
        .ok_or(E_NO_CONSOLE)?;

    // 4：行规程那一圈，直到收场词 / EOF。
    adapt::terminal::run(&mut console);
    Ok(())
}
