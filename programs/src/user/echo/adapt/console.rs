//! echo::adapt::console — **找控制台**：`/device/uart/{rx,tx}` 那两枚门牌。
//!
//! 那一趟（名字 → 号 → 入口）与它那一圈"再问一次"的重试全在 [`operator::entry_of`] 里
//! （"门牌由别的域落下，本域可能比它先起"）；本文件只剩"往哪找"这一格：目录是驱动族那一段，
//! 再下一段是服务名，最后一段是**两面**（[`RX`] 读 / [`TX`] 写）。
//!
//! 找到之后那两枚**都从会话里**进本域表，而**它们在本域表里的号随答话回来**
//! （`ocall::Union::Seed`）——故这一趟不必认"哪一份"，号就是这一趟自己那一枚（次序那件事见
//! `user/echo/mod.rs` 的照实记）。**缺任一枚都算没找到**：读得到、写不出去的回显没有意义。

use super::{MS, RX, TX, WANT};
use env::{Name, Wait};
use protocol::communication::session::Session;
use protocol::driver::DIR;
use protocol::system::operator::client as operator;
use runtime::env::mail::HolePie;

/// 控制台那两面：[`rx`](Self::rx) = 本域取（控制台排空出来的一批），
/// [`tx`](Self::tx) = 本域推（一条完整的字，控制台写进设备）。
pub struct Console {
    pub rx: HolePie,
    pub tx: HolePie,
}

/// 找控制台（有界）：任一面的门牌找不到 ⇒ `None`（`main` 据此报
/// [`E_NO_CONSOLE`](super::E_NO_CONSOLE)）。
pub fn find(session: &Session) -> Option<Console> {
    let (Ok(dir), Ok(want), Ok(rx), Ok(tx)) = (
        Name::new(DIR),
        Name::new(WANT),
        Name::new(RX),
        Name::new(TX),
    ) else {
        return None;
    };
    let rx = operator::entry_of(session, &[dir, want, rx], Wait::AtMost(MS)).ok()?;
    let tx = operator::entry_of(session, &[dir, want, tx], Wait::AtMost(MS)).ok()?;
    Some(Console {
        rx: HolePie::from_token(rx),
        tx: HolePie::from_token(tx),
    })
}
