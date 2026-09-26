//! echo::adapt::board — **上板报到**（与 `passer` 同一段前奏）：返板的答码（`bcall::OK` = 挂上了）。
//!
//! 挂不上照旧回显——只是"本域死了"那条信号缺席（见 `user/echo/mod.rs`）。

use super::{ME, MS};
use env::{Name, Wait};
use protocol::system::board as bcall;
use protocol::system::board::client as board;
use runtime::env::mail;
use runtime::env::unit as utask;

/// 把本域的入口经会话交给板（挂上牌子）。
pub fn register() -> u8 {
    let sire = utask::sire();
    let Ok((link, board)) = board::open(sire, Wait::AtMost(MS)) else {
        return bcall::BAD;
    };
    let Ok(talk) = board::ask_hole(board) else {
        return bcall::BAD;
    };
    let Ok(entry) = mail::unseal_hole(board::ENTRY_MARK) else {
        return bcall::BAD;
    };
    let Ok(me) = Name::new(ME) else {
        return bcall::BAD;
    };
    board::register(talk, &link, board, me, entry, Wait::AtMost(MS)).unwrap_or(bcall::BAD)
}
