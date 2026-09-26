//! Sender — **我推的那一枚孔**：这一路流的那一种报由类型参数说。
//!
//! ```text
//!   Sender::send(报, 期限)    编进本族的缓冲（**在这一帧的栈上**）→ 推上这一枚孔
//! ```
//!
//! # 三条照实记
//!
//! **① 一枚孔一个方向。** Mail 是单向单槽 ⇒ "收发"不是一枚孔上的两件事，而是**两枚孔、
//! 两个对象**（本文件与 [`Receiver`](super::receiver)）。这一枚孔是谁铸的、谁读的，由
//! **建立那一步**说（[`super::establish`]），本文件不问。
//!
//! **② 期限在每次调用上**（与 `std::sync::mpsc` 的 `recv_timeout` 同构）：`Wait` 一个参数
//! 说尽三态——`POLL`（= `AtMost(0)`）**就是** `try_send`：单次尝试、槽满当场答 `Busy`，
//! 一次也不挂起。
//!
//! **③ 缓冲在这一帧的栈上，且循环要自己写。** 发出去的报是**自己编的**，超不出本族最长
//! 那一枚（`M::Buf` 就是它）⇒ 既不占调用方的缓冲、也不占结构体的字段。而
//! **`HolePie::push` 把等待写死成 `Wait::Forever`**（`runtime/src/env/mail.rs` 那一圈）⇒
//! `POLL` 与 `AtMost` 在它那里落不下来；故下面这一手用 `env::mail::push` ＋
//! `HolePie::wait(HoleDir::Push, …)` 把那一圈重写一遍（原 `session::call::push_to` /
//! `try_post` 就是这两态各自一副身体，现在收成同一条路上的三态）。

use core::marker::PhantomData;

use env::{HoleDir, MailFail, PieToken, Wait};

use super::{deadline, remain};
use crate::message::Message;
use runtime::env::mail;

/// **我推的那一枚孔** ＋ 这一路流的那一种报（类型）。
pub struct Sender<M: Message> {
    hole: Option<PieToken>,
    _m: PhantomData<M>,
}

impl<M: Message> Sender<M> {
    /// 认下一枚**别人给的**号（服务端那一侧：孔是对方铸的、交给我的）。
    ///
    /// **本文件不分辨"这枚是谁的"**——归属归建立那一手返的那一对（[`super::establish::Pair`]）：
    /// 那一对里两枚都是本端铸的，收尾时放下；这一手拿到的**不归本端**，放下它不是本端的事
    /// （放了就把客人的孔收掉）。
    pub fn from_token(hole: PieToken) -> Self {
        Self {
            hole: Some(hole),
            _m: PhantomData,
        }
    }

    /// **没有写端**的那一只（对端那一枚没认到）。
    ///
    /// **这不是错误**：一段关系可以只有收的方向——单向那一档就是这么用的。
    /// `send` 对它答 [`SendFail::Unbound`]。
    pub(crate) fn unbound() -> Self {
        Self {
            hole: None,
            _m: PhantomData,
        }
    }

    /// 编 ＋ 推。`wait` = 槽满等多久（**三态**，见文件头 ②）。
    ///
    /// 失败三层**分得开**（[`SendFail`]）：编不下（`TooLong`）/ 没有写端（`Unbound`）/
    /// 搬不动（`Mail`）。
    pub fn send(&self, msg: M, wait: Wait) -> Result<(), SendFail> {
        let Some(hole) = self.hole else {
            return Err(SendFail::Unbound);
        };
        let mut buf = M::EMPTY;
        let Some(n) = msg.store(buf.as_mut()) else {
            return Err(SendFail::TooLong);
        };
        // **长度也归这一格管**：`store` 报的比 `Buf` 还大时不能拿它去切——那是"编出来的字节
        // 说了谎"，与"装不下"同一条下场（原 `Sender::send` 的 `debug_assert` 在这里落成返回值）。
        let bytes = buf.as_ref().get(..n).ok_or(SendFail::TooLong)?;
        push(hole, bytes, wait).map_err(SendFail::Mail)
    }

    /// 这一枚孔（诊断、挂进组、转授都从这里取）。**没有写端时答 `None`**。
    pub fn hole(&self) -> Option<PieToken> {
        self.hole
    }
}

/// 推不出去：三层**分得开**。
///
/// - [`SendFail::TooLong`] = **编不进本族的缓冲**（`M::Buf` 就是本族最长那一枚，故这一支只在
///   类型被写错时才到得了——不 `panic`、如实报）；
/// - [`SendFail::Unbound`] = **没有写端**（对端那一枚还没认到）；
/// - [`SendFail::Mail`] = **搬不动**，原样的域词汇（`Busy` / `Dead` / `Denied` / …）。
///
/// **不另造一套码**：Mail 域的词表是它的失败域，这一层只把"哪一步失败"说清，不换词。
pub enum SendFail {
    Unbound,
    TooLong,
    Mail(MailFail),
}

/// 推一串字节，按 `wait` 的重试。
fn push(hole: PieToken, bytes: &[u8], wait: Wait) -> Result<(), MailFail> {
    // **`POLL` = 单次尝试**（`try_send`）：一次也不挂起。
    if wait == Wait::POLL {
        return mail::push(hole, bytes.as_ptr(), bytes.len()).map_err(|e| e.source);
    }

    let until = deadline(wait);
    let pie = mail::HolePie::from_token(hole);
    loop {
        match mail::push(hole, bytes.as_ptr(), bytes.len()) {
            Ok(()) => return Ok(()),
            // 槽满：睡到有空间再来（下面那一手）。
            Err(e) if e.source.is_busy() => {}
            Err(e) => return Err(e.source),
        }
        let remain = remain(until);
        if remain == Wait::POLL {
            // 期限内没腾出槽位：这一格答**忙**，不是"孔坏了"。
            return Err(MailFail::Busy);
        }
        // **醒来自己再推一次**：这一手只是提示（真醒还是期限到，由下一轮那一推说了算）。
        let _ = pie.wait(HoleDir::Push, remain);
    }
}
