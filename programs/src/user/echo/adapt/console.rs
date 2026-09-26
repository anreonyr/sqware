//! echo::adapt::console — **找控制台**：`FIND /device/uart`，**找不到就再问**（有界）。
//!
//! 找到之后那一枚**从会话里**进本域表，而**它在本域表里的号随答话回来**（`ocall::Union::Seed`）
//! ——故这一趟不必认"哪一份"，号就是这一趟自己那一枚（次序那件事见 `user/echo/mod.rs` 的照实记）。

use super::{MS, RETRY_MS, WANT};
use core::time::Duration;
use env::{Name, PieToken, Wait};
use protocol::driver::DIR;
use protocol::session::Quay;
use protocol::system::operator as ocall;
use protocol::system::operator::client as operator;
use runtime::env::mail::HolePie;
use runtime::env::room;

/// 找控制台（有界）：找不到 ⇒ `None`（`main` 据此报 [`E_NO_CONSOLE`](super::E_NO_CONSOLE)）。
pub fn find(link: &Quay, talk: PieToken) -> Option<HolePie> {
    let (Ok(dir), Ok(want)) = (Name::new(DIR), Name::new(WANT)) else {
        return None;
    };
    let road = [dir, want];
    // **间接寻址那一手**：名字先经 `seek` 译成号（"还没挂上"那一格也在这里重试），此后按号。
    //
    // **总预算就是 `MS`**（照实记：原来每一趟都按写死的 `MS` 问，而那一趟自己就能花掉 `MS`
    // ⇒ "预算"实际是"重试次数 × MS"）。故把**剩下的那点预算**当这一趟的期限递下去。
    let mut left = MS;
    let id = loop {
        match operator::seek(talk, link, &road, Wait::AtMost(left)) {
            Ok(id) => break id,
            Err(ocall::UNKNOWN) if left > 0 => {
                let _ = room::sleep(Duration::from_millis(RETRY_MS as u64));
                left = left.saturating_sub(RETRY_MS);
            }
            Err(_) => return None,
        }
    };
    match operator::find(talk, link, id, Wait::AtMost(MS)) {
        Ok((ocall::OK, Some(entry))) => Some(HolePie::from_token(entry)),
        _ => None,
    }
}
