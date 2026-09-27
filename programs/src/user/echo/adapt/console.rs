//! echo::adapt::console — **找控制台**：`/device/uart` 那扇门。
//!
//! 那一趟（名字 → 号 → 入口）与它那一圈"再问一次"的重试全在 [`Session::service`] 里
//! （"门牌由别的域落下，本域可能比它先起"——见 `programs/src/session.rs` 头注）；本文件
//! 只剩"往哪找"这一格：目录是驱动族那一段，名字是服务名。
//!
//! 找到之后那一枚**从会话里**进本域表，而**它在本域表里的号随答话回来**（`ocall::Union::Seed`）
//! ——故这一趟不必认"哪一份"，号就是这一趟自己那一枚（次序那件事见 `user/echo/mod.rs` 的照实记）。

use super::{MS, WANT};
use env::Wait;
use programs::session::Session;
use protocol::driver::DIR;
use runtime::env::mail::HolePie;

/// 找控制台（有界）：找不到 ⇒ `None`（`main` 据此报 [`E_NO_CONSOLE`](super::E_NO_CONSOLE)）。
pub fn find(session: &Session) -> Option<HolePie> {
    session
        .service(DIR, WANT, Wait::AtMost(MS))
        .ok()
        .map(HolePie::from_token)
}
