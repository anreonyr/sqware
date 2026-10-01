//! Sender — **我推的那一枚孔**：这一路流的那一种报由类型参数说。
//!
//! ```text
//!   Sender::send(报)     编进本族的缓冲（**在这一格自己身上**）→ 递上这一枚孔（不睡）
//!   Sender::reclaim()    等那只手被取走（还挂着的时候）
//!   Drop                 落出作用域时自动收口——**忘不了**
//! ```
//!
//! # 四条照实记
//!
//! **① 一枚孔一个方向。** Mail 是单向单手 ⇒ "收发"不是一枚孔上的两件事，而是**两枚孔、
//! 两个对象**（本文件与 [`Receiver`](super::receiver)）。这一枚孔是谁铸的、谁读的，由
//! **建立那一步**说（[`super::establish`]），本文件不问。
//!
//! **② 期限不在这里，也不在载体那一层**（照实记：本刀之前 `runtime` 里有一格
//! `HANDOFF_MS = 1000`——递出去之后等对面来取，等满就**把那一条撤掉**、答 `Busy`）。
//! 那一格实测把"对面慢"折成了"这条报作废"：驱动起手向设备账认领那一趟就此失败，接着
//! 装配失败、级联（评判与读数见 `runtime::env::mail` 文件头那条照实记）。今天的口径是
//! **不到手不罢休**：`send` 只递（`Wait::POLL`，一个 envcall），[`Sender::reclaim`] 等它下线，
//! 唯一的"不"是孔封印（`Dead`）。**撤手那一格（`Withdraw`）不存在**——用户裁定"不留"。
//!
//! **③ 缓冲住在这一格身上，而不是借别人的栈帧。** 发出去的报是**自己编的**，超不出本族最长
//! 那一枚（`M::Buf` 就是它）⇒ 随 `Sender` 一起内联，不分配。**为什么必须由发送方持有**：那只手
//! 记的是登记那一刻的**地址**，所以那段字节一步都不许搬（照实记：先前那一版把"字节"与"那只手"
//! 一起包成一个值从 `send` **返回**，于是它在返回、入槽、赋值这几处被**搬动**，而手还指着搬家
//! 前那片栈——qtest 上侥幸对（那片栈还没被覆盖），**直接起 QEMU 那一路当场 `supply: recv
//! unread`**，对端复制到的是一段陈地址）。今天字节就在这一格身上，`send` 只借 `&mut self`，
//! 地址在整段借用期里不动，由编译器保证。
//!
//! **④ "忘不了"由 `Drop` 兜底。** 照实记（一处真洞）：`supply::client::draw` 在 `send` 与收口之间
//! 有三条早退 `return Err(...)`，旧名字那版每一支都漏掉收口 ⇒ 那段栈一死、孔上还挂着指它的手，
//! 对端来取就复制到一段别人的栈。今天 `Drop` 把这一笔接走：**任何一条路径**（早退、`?`、panic
//! 展开）都收口。这也是"载体不替调用方等、由写端显式等"能成立的前提——等这件事有人兜。
//!
//! **`Outbox`（旧名字）并进了本类型**：它当年只是"写端 ＋ 缓冲 ＋ 那只手"三格的另一种摆法，
//! 于是同一个孔上会同时存在两个写端（`Sender` 与 `Outbox`），"一格招待所有客人"那种错**编得过**。
//! 今天一个孔、一个消息族只有**一枚** `Sender`。

use core::marker::PhantomData;

use env::{HoleDir, MailFail, MailResult, PieToken, Wait};

use crate::message::Message;
use runtime::env::mail::HolePie;

/// **我推的那一枚孔** ＋ 这一路流的那一种报（类型）＋ 那一格缓冲 ＋ 还挂着的那只手。
pub struct Sender<M: Message> {
    /// 写端那一枚（`None` = 还没有）。
    hole: Option<PieToken>,
    /// 编报那一格：地址在整个借用期里不动（见文件头③）。
    buf: M::Buf,
    /// **还挂在孔上的那只手**（`None` = 空着）。
    hand: Option<PieToken>,
    _m: PhantomData<M>,
}

impl<M: Message> Sender<M> {
    /// 空格：**没有写端**（`send` 会答 [`SendFail::Unbound`]）。`const` 是给"放进结构体里当一格"
    /// 那些用到上的（`Guest` 那一格）。
    pub const fn new() -> Self {
        Self {
            hole: None,
            buf: M::EMPTY,
            hand: None,
            _m: PhantomData,
        }
    }

    /// 认下一枚**别人给的**号（服务端那一侧：孔是对方铸的、交给我的）。
    ///
    /// **本文件不分辨"这枚是谁的"**——归属归建立那一手返的那一对（[`super::establish::Endpoint`]）：
    /// 那一对里两枚都是本端铸的，收尾时放下；这一手拿到的**不归本端**，放下它不是本端的事
    /// （放了就把客人的孔收掉）。
    pub fn from_token(hole: PieToken) -> Self {
        Self {
            hole: Some(hole),
            buf: M::EMPTY,
            hand: None,
            _m: PhantomData,
        }
    }

    /// 编 ＋ 递：**手递出去就返回**（不睡）——字节住在这一格身上，`Drop` 保证收口。
    ///
    /// **孔上站着别人的手 ⇒ `SendFail::Mail(Busy)`**（单槽孔的那一格）：要"等轮到自己"就用裸的
    /// [`HolePie::push`] 把预算写出来（门面客侧就是这么写的，照实记见 `HolePie::push`）——
    /// 本手不做那一等，因为**服务端回话**与**客侧敲门**对"等"的态度正相反：回话那一侧一睡，
    /// 一台服务就被一位慢客人卡住，而这一格的所有用家都是回话那一侧。
    ///
    /// 失败两层**分得开**（[`SendFail`]）：编不下（`TooLong`）/ 没有写端（`Unbound`）/
    /// 递不动（`Mail`）。
    pub fn send(&mut self, msg: M) -> Result<(), SendFail> {
        let Some(hole) = self.hole else {
            return Err(SendFail::Unbound);
        };
        // 复用这一格之前先把上一手收口（**这一等不在"递出去"那一段上**）。
        self.reclaim().map_err(|e| SendFail::Mail(e.source))?;
        let Some(n) = msg.store(self.buf.as_mut()) else {
            return Err(SendFail::TooLong);
        };
        // **长度也归这一格管**：`store` 报的比 `Buf` 还大时不能拿它去切——那是"编出来的字节
        // 说了谎"，与"装不下"同一条下场。
        let bytes = self.buf.as_ref().get(..n).ok_or(SendFail::TooLong)?;
        HolePie::from_token(hole)
            .push(bytes, Wait::POLL)
            .map_err(|e| SendFail::Mail(e.source))?;
        self.hand = Some(hole);
        Ok(())
    }

    /// **等这只手下线**：送到（孔回到空）或孔封印（`Err(Dead)`）或孔不见了（`Gone`）。
    /// **没有期限**——见文件头②。已经空着 ⇒ 当场 `Ok`（零代价）。
    ///
    /// **照实记（"孔已不在本表"那一格是正常终局，不是故障）**：对面把那一枚收了（`seal` ＋
    /// `release`，见 `programs/src/system/desk.rs` 那一族）⇒ **级联**把我这份副本一起摘了，
    /// 于是这一等答 `Denied`。按构造它安全：对面收摊之前**必然已经取走那只手**（或它 `seal` 了，
    /// 那一只手就地抹掉）。
    ///
    /// **但陷阱长着同一张脸**——**我自己**先 `release` 再收口时，孔还活着，那只手**还指着这段
    /// 缓冲**（症状：客侧 `recv-unread`，内核 `hand_over` 读数一切正常）。故这一格留一行**调试面**
    /// 读数（`debug!`：**release 下不打** —— 这一格每跑几百回，正常终局不该占 release 的串口）。
    pub fn reclaim(&mut self) -> MailResult<()> {
        let Some(hole) = self.hand.take() else {
            return Ok(());
        };
        let r = HolePie::from_token(hole).wait(HoleDir::Push, Wait::Forever);
        if let Err(e) = &r {
            crate::debug!(
                "mail: reclaim miss hole={} code={}",
                hole.get(),
                e.source.code()
            );
        }
        r.map(|_| ())
    }

    /// **非阻塞收口**：上一只手**已经被取走** ⇒ 放下那一格、答 `true`；还压着 ⇒ 答 `false`。
    ///
    /// **它与 [`reclaim`] 只差一个字：期限。** `reclaim` 等 `Forever`——写端**必须**替这段缓冲收尾
    /// （缓冲在这一格身上，见文件头③）；而这一手**一眼都不多等**。
    ///
    /// **它只对一类用家成立**：一枚线程招待所有客人那种服务（`programs/src/system/operator/
    /// server.rs` 的 `Outbox`）。那一类里"等一位客人把答话取走"这一等**就落在服务循环里**——
    /// 实测：客人 24 不来取，本域在那一等上停了 **4639 ms**，那段时间里客人 25 / 26 的手在孔上
    /// 干等（各 2.1 s，两位当场判失败）。
    ///
    /// **用它的一侧必须把缓冲放在比这一趟长的地方**：`send` 编的是 `self.buf`，而那只手记的是
    /// 编那一刻的地址——缓冲随这一趟的栈帧死掉，取的人就复制到别人的字节（文件头③那条洞）。
    /// 答 `false` 时**那只手还挂着**：不许再 `send`（会把那只手指着的字节改掉），只能等下一次。
    pub fn settle(&mut self) -> bool {
        let Some(hole) = self.hand else {
            return true;
        };
        if let Ok(true) = HolePie::from_token(hole).wait(HoleDir::Push, Wait::AtMost(0)) {
            self.hand = None;
            return true;
        }
        false
    }

    /// 还挂着吗（诊断：不碰内核就能问）。
    pub const fn outstanding(&self) -> bool {
        self.hand.is_some()
    }

    /// 这一枚孔（诊断、挂进组、转授都从这里取）。**没有写端时答 `None`**。
    pub fn hole(&self) -> Option<PieToken> {
        self.hole
    }
}

impl<M: Message> Default for Sender<M> {
    fn default() -> Self {
        Self::new()
    }
}

/// **落出作用域 = 收口**：还挂着那只手就等它下线。
///
/// 这是"递出即走"能安全成立的那一半（见文件头④）：`send` 不睡，代价是"这段字节还欠着"，而
/// 这一格的 `Drop` 就是那一笔的收尾。
///
/// **照实记（这一格有一个陷阱，量出来的）**：这一等用的是**调用方表里那一枚孔**
/// （`wait(HoleDir::Push)` 要走权限那一关）⇒ **放下那一枚必须在收口之后**。先
/// `mail::release(back)`、再让这一格落出作用域，那一等当场答 `Denied`，而孔上那只手**还指着
/// 这一帧的栈**：函数一返回，下一趟复用同一片栈，取的人复制到的是**别人的字节**。症状是客侧
/// `recv-unread`（长度与发送者都对、内容不对），而内核 `hand_over` 那条读数一切正常——
/// `principal` / `coalition` / `control` 三处服务面因此都用**一层作用域**把收口钉在 `release`
/// 之前（见各 `turn` 的照实记）。收口失败时 [`Sender::reclaim`] 会留一行读数。
///
/// **它真的会等**：若对侧永不取走且不封印，这里会停住；故**读者那一侧必须结清**（弃读就
/// `seal`／`release`，见 `programs/src/system/desk.rs` 那一族），否则这一等就成了新的卡点。
impl<M: Message> Drop for Sender<M> {
    fn drop(&mut self) {
        if self.hand.is_some() {
            let _ = self.reclaim();
        }
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
