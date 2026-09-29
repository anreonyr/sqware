//! Receiver — **我收的那一枚孔**：这一路流的那一种报由类型参数说。
//!
//! ```text
//!   Receiver::recv(缓冲, 期限)    从这一枚孔取一串字节 → 解成本族那一形
//! ```
//!
//! # 缓冲由**调用方**给（与 [`Sender`](super::sender) 相反）
//!
//! 发那一侧可以自己带：那条报是自己编的，超不出本族最长（`M::Buf`）。收这一侧不行——
//! **对面推得进来什么，孔不预设**（内核不再有"一条消息 ≤ 一页"那条界），而 `M::Buf` 只保证
//! "本族合法的最长那一枚"。比 `M::Buf` 长的那一条：核答 `Denied` 而**手原样**（丢一条消息
//! 不可逆）——拿 `M::Buf` 收就是读到一个"读不懂"，可那一条**还留在孔上** ⇒ 组每轮都唤醒、
//! 门每轮答一句 `BAD`，而这位客人下一次正经的推**堵在门外**（照实记见
//! `harness/src/probe_bound.rs`）。故凡"门"那一侧收帧都备**够大**的缓冲（本仓给一页，
//! 远大于任何一族的最长帧）；**客侧那一枚孔只有本族对面会推** ⇒ 拿本族那只空缓冲
//! （`Message::EMPTY`）就够。
//!
//! # 期限在**每次调用**上
//!
//! 同 `std::sync::mpsc` 的 `recv_timeout`：`POLL`（= `AtMost(0)`）**就是** `try_recv`。

use core::marker::PhantomData;

use env::{MailFail, PieToken, Wait};

use crate::message::Message;
use runtime::env::mail;

/// **我收的那一枚孔** ＋ 这一路流的那一种报（类型）。
pub struct Receiver<M: Message> {
    hole: PieToken,
    _m: PhantomData<M>,
}

impl<M: Message> Receiver<M> {
    /// 认下一枚**别人给的**号（服务端那一侧：孔是对方铸的、交给我的）。
    ///
    /// **本文件不分辨"这枚是谁的"**——归属归建立那一手返的那一对；这一手拿到的**不归本端**，
    /// 放下它不是本端的事（放了就把客人的孔收掉）。
    pub fn from_token(hole: PieToken) -> Self {
        Self {
            hole,
            _m: PhantomData,
        }
    }

    /// 收 ＋ 解。`wait` = 等多久（`POLL` = 只探测）。
    ///
    /// 失败三格**分得开**（[`RecvFail`]）：搬不动（`Mail`）/ 收到了解不动（`Unread`）——
    /// 而 `Mail` 里 `Busy`（期限内没等到）与 `Dead` / `Denied`（这一枚孔用不动了）也分得开。
    pub fn recv(&self, buffer: &mut [u8], wait: Wait) -> Result<M::In, RecvFail> {
        let n = mail::HolePie::from_token(self.hole)
            .pull_timeout(buffer, wait)
            .map_err(|e| RecvFail::Mail(e.source))?;
        // 短一字节即读不懂；缓冲比帧还短也落这一格（那一格按本族最长给时到不了）。
        let bytes = buffer.get(..n).ok_or(RecvFail::Unread)?;
        M::fetch(bytes).ok_or(RecvFail::Unread)
    }

    /// 这一枚孔（**挂进组**用：一台驱动要同时等"线上有投递"与"门上有人"）。
    ///
    /// 与 [`Receiver::recv`] 读的是同一枚——组等的是**就绪**，取消息仍走 `recv`。
    pub fn hole(&self) -> PieToken {
        self.hole
    }
}

/// 收不回来：三格**分得开**。
///
/// - [`RecvFail::Unread`] = **收到了、解不动**（长度不对 / 形状不对 / 缓冲比帧还短）。
///   **它不属于载体那一层**：搬字节的那一手不做解码，故"读不懂"在这一格、不在 [`MailFail`] 里；
/// - [`RecvFail::Mail`] = **搬不动**，原样的域词汇。这里 **`Busy`（期限内没等到）与
///   `Dead` / `Denied`（这一枚孔用不动了）分得开**——"再等等"与"别等了"是两个下一步：
///   `root` 的发货循环靠这一格决定"没收到 ⇒ 去探一次对端活没有"还是"孔用不动 ⇒ 收摊"。
///
/// （原 `Receiver::recv` 那三格 `Expired` / `Unavailable` / `Unread` 就是这三件；`Expired` 与
/// `Unavailable` 此前是**折出来的**，今天直接读 `MailFail` 的两族词，不再有中间那张对照表。）
pub enum RecvFail {
    Mail(MailFail),
    Unread,
}
