//! Sender — **我推的那一枚孔**：这一路流的那一种报由类型参数说。
//!
//! ```text
//!   Sender::send(报)    编进本族的缓冲（**在这一帧的栈上**）→ 递上这一枚孔 → 送到
//! ```
//!
//! # 三条照实记
//!
//! **① 一枚孔一个方向。** Mail 是单向单手 ⇒ "收发"不是一枚孔上的两件事，而是**两枚孔、
//! 两个对象**（本文件与 [`Receiver`](super::receiver)）。这一枚孔是谁铸的、谁读的，由
//! **建立那一步**说（[`super::establish`]），本文件不问。
//!
//! **② 期限不在这里**（照实记：本刀之前这里有一个 `wait: Wait` 参数，与一圈手写的三态循环）。
//! 孔上那一格是**一只手**，不是一只槽：递出去只有一个下场——**送到**。仓里 24 处调用点
//! **全都传 `Wait::Forever`** ⇒ 那一格没有选择者，它不是参数、是常量，整格退场。要非阻塞
//! 投递（`try_send`）得先有"撤手"那一格（见 `MailCall::Push` 的契约），那是另一单。
//!
//! **送的"等"有一个兜底期限**（`runtime::env::mail` 的 `HANDOFF_MS`）：对面不在（走了、或没在
//! 收）⇒ 到点撤手、答 `Busy`，**不永久挂着**。判死只有孔那一格状态轴（`MailCall::Push` 的
//! `Dead`）——而"孔还在、人没了"那一类**正是靠退场那次封印把它变成 `Dead`**（钩子直接收
//! `&Arc<Task>`，见 `messenger::Hook`），不是靠发送方在这里猜；期限只兜"人还在、只是没来取"。
//!
//! **③ 缓冲在这一帧的栈上，且借用期覆盖"递出 → 送到"两段。** 发出去的报是**自己编的**，
//! 超不出本族最长那一枚（`M::Buf` 就是它）⇒ 既不占调用方的缓冲、也不占结构体的字段；
//! 而孔上那只手指的就是这片栈——`send` 不返回，它就悬不了。

use core::marker::PhantomData;

use env::{MailFail, PieToken};

use crate::message::Message;
use runtime::env::mail::HolePie;

/// **我推的那一枚孔** ＋ 这一路流的那一种报（类型）。
pub struct Sender<M: Message> {
    hole: Option<PieToken>,
    _m: PhantomData<M>,
}

impl<M: Message> Sender<M> {
    /// 认下一枚**别人给的**号（服务端那一侧：孔是对方铸的、交给我的）。
    ///
    /// **本文件不分辨"这枚是谁的"**——归属归建立那一手返的那一对（[`super::establish::Endpoint`]）：
    /// 那一对里两枚都是本端铸的，收尾时放下；这一手拿到的**不归本端**，放下它不是本端的事
    /// （放了就把客人的孔收掉）。
    pub fn from_token(hole: PieToken) -> Self {
        Self {
            hole: Some(hole),
            _m: PhantomData,
        }
    }

    /// 编 ＋ 递。**送到才算完**（阻塞；见文件头 ②）。
    ///
    /// 失败两层**分得开**（[`SendFail`]）：编不下（`TooLong`）/ 没有写端（`Unbound`）/
    /// 递不动（`Mail`）。
    pub fn send(&self, msg: M) -> Result<(), SendFail> {
        let Some(hole) = self.hole else {
            return Err(SendFail::Unbound);
        };
        let mut buf = M::EMPTY;
        let Some(n) = msg.store(buf.as_mut()) else {
            return Err(SendFail::TooLong);
        };
        // **长度也归这一格管**：`store` 报的比 `Buf` 还大时不能拿它去切——那是"编出来的字节
        // 说了谎"，与"装不下"同一条下场。
        let bytes = buf.as_ref().get(..n).ok_or(SendFail::TooLong)?;
        HolePie::from_token(hole)
            .push(bytes)
            .map_err(|e| SendFail::Mail(e.source))
    }

    /// 这一枚孔（诊断、挂进组、转授都从这里取）。**没有写端时答 `None`**。
    pub fn hole(&self) -> Option<PieToken> {
        self.hole
    }
}

/// 递不出去：两层**分得开**。
///
/// - [`SendFail::TooLong`] = **编不进本族的缓冲**（`M::Buf` 就是本族最长那一枚，故这一支只在
///   类型被写错时才到得了——不 `panic`、如实报）；
/// - [`SendFail::Unbound`] = **没有写端**（对端那一枚还没认到）；
/// - [`SendFail::Mail`] = **搬不动**，原样的域词汇（`Busy` / `Dead` / `Denied` / `Gone`）。
///
/// **不另造一套码**：Mail 域的词表是它的失败域，这一层只把"哪一步失败"说清，不换词。
pub enum SendFail {
    Unbound,
    TooLong,
    Mail(MailFail),
}
