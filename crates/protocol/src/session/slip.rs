//! **船台** —— 一条泊位上的端点：一枚孔 ＋ **这条路上流的那一种报**（类型绑定）。
//!
//! ```text
//!   Slip::seal(pie)                 认下一枚孔 ⇒ 一个船台（类型在这儿绑上）
//!         .load(m)                  装上一条报（编进船台自己那只缓冲）；编不上 ⇒ Err(那条报)
//!         .ship()                   发出去
//!
//!   slip.land(buf, millis)          收一条报（缓冲由调用方给；失败两格见 [`Land`]）
//! ```
//!
//! # 为什么它是一层库
//!
//! 四个方法（＋ [`Land`] 那两格失败）只碰**两枚原语**（`HolePie::push` / `pull_timeout`）与
//! **一条约定**（[`Message`]），与哪一族、哪条路、什么荷载全无关。它住 `protocol`
//! 是因为只有这一层同时看得见"孔"（`runtime`）与"报"（`contract`）。
//!
//! # 本仓每一枚孔是**单向**的
//!
//! 问与答走两枚孔（板那一族的问话孔与答话路、树那一族的问话孔与树路），故一个船台只用一个
//! 方向：**发的那侧**用 `load` / `ship`，**收的那侧**用 `land`；`M` 就是那个方向流的报。
//!
//! # 两只缓冲：发的那只船台自己带，收的那只调用方给
//!
//! 发出去的报是**自己编的**，超不出本族最长 ⇒ 船台自己带一只（`seal` 时就是空的）。收进来的
//! 报**由载体定界**（谁推得进来、一条多长，不由本族说）⇒ 缓冲由调用方给。理由与实测写在
//! [`Slip::land`] 上。
//!
//! # 照实记：三个动词与仓里已有的同词不同事（用户裁定：就用这三个）
//!
//! - [`Slip::seal`] ←→ `runtime::env::pie::seal`：那个是**封印一枚孔**，这一手是**由一枚孔造出
//!   船台**；
//! - [`Slip::ship`] ←→ `port::ship` / `session::call::ship` / …：那些是**授一枚门闩**，这一手是
//!   **把一条报推出去**；
//! - [`Slip::land`] ←→ `Ledger::land` / `Operator::land`：那些是**落一格账 / 落一枚入口**，
//!   这一手是**从孔里收一条报**。
//!
//! 三句各自写在方法上，就地声明区分（本仓对同词的既有做法，见 `system` 那一份的 `Unit` 注）。

use core::marker::PhantomData;

use contract::message::Message;
use env::Wait;
use runtime::env::mail::HolePie;

/// 一枚孔上的端点：**这条路上流的那一种报**由类型参数说。
pub struct Slip<M: Message> {
    pie: HolePie,
    buf: M::Buf,
    len: usize,
    _m: PhantomData<M>,
}

impl<M: Message> Slip<M> {
    /// 认下一枚孔 ⇒ 一个船台。
    ///
    /// **照实记（同词不同事）**：`runtime::env::pie::seal` 是"封印一枚孔"，这一手是"由一枚孔
    /// 造出船台"。
    pub fn seal(pie: env::PieToken) -> Self {
        Self {
            pie: HolePie::from_token(pie),
            buf: M::EMPTY,
            len: 0,
            _m: PhantomData,
        }
    }

    /// 装上一条报（编进自己那只缓冲）：**编不进 ⇒ 把消息原样交回**。
    ///
    /// **照实记（它替掉了什么）**：原先是
    /// ```ignore
    /// self.len = m.store(self.buf.as_mut()).unwrap_or(0);
    /// ```
    /// ⇒ 编码失败把长度记成 0，[`Slip::ship`] 于是往孔里推一条 **0 字节帧**（内核答 `Denied`），
    /// 真因被伪装成"对面坏了"。这一支按构造到不了（`Buf` 就是本族最长的那一枚），
    /// **但"到不了"不等于"可以不报"**：今天它报得出来——消息原样交回。
    ///
    /// **照实记（`Err` 那一格带的是消息、不是失败码）**：这一层不认识任何一族的失败域
    /// （见文件头的分层），而"没装进去"这件事的全部内容就是那条消息本身；调用方按自己的
    /// 失败域解释它（今天各处折成 `Denied` 之类的"这一手没做成"）。
    ///
    /// **照实记（这一格的保证是什么、不是什么）**：本文件住 `protocol`，后者拖 `runtime`
    /// （riscv 内联汇编、无 `cfg` 护栏）⇒ 这一支**宿主上链接不到、更判不了**。它的保证是
    /// **类型上到不了**：`Buf` 由本族 `Message` 自己给。故这里不写 `expect`、不写
    /// `unwrap_or`，只留一条读得见的返回值。
    pub fn load(self, m: M) -> Result<Self, M> {
        let mut buf = self.buf;
        let Some(len) = m.store(buf.as_mut()) else {
            return Err(m);
        };
        // **长度也归这一格管**：`store` 报的比 `Buf` 还大时，`ship` 那一刀会把它当"整只缓冲"
        // 切——那是**编出来的字节说了谎**，与"装不下"同一条下场。
        debug_assert!(
            len <= buf.as_ref().len(),
            "Message::store 报的长度超出了它自己的 Buf"
        );
        if len > buf.as_ref().len() {
            return Err(m);
        }
        Ok(Self {
            pie: self.pie,
            buf,
            len,
            _m: PhantomData,
        })
    }

    /// 发出去。
    ///
    /// **照实记（同词不同事）**：`port::ship` 那一族是"授一枚门闩"，这一手是"把一条报推出去"。
    ///
    /// **照实记（返回的是 [`env::Fail`]——这一格绕了一刀）**：裁的那一版写的就是它，而刀一
    /// 落盘时返的是 `EnvResult`——那时仓里**没有**"码 → 词汇"那道读法（`Fail` 是**词汇**，
    /// `EnvError` 是**对线的读法**），硬折成某一格会把 `Busy`（槽满）与 `Dead`（孔没了）都说成
    /// 别的失败。刀 C 在 `env` 立了 [`env::Fail::of_code`] 之后，这一手按裁决收成词汇。
    ///
    /// **照实记（表外的码 ⇒ [`env::Fail::Denied`]）**：表里那七枚都是**读出来**的，本层
    /// **唯一一处任意**是表外那一格。表外的码是"内核报了一个我这一版不认识的失败"，总得落
    /// 一格；`Denied` 是"这一手没做成"里**最不含承诺**的那一枚（不说"槽满"、不说"孔没了"）。
    /// **不假装它是真的**：想分辨这一种情形的调用方得自己拿原样的 `EnvError`——那一层今天
    /// 没有读者，故这一手不外送它。
    pub fn ship(self) -> Result<(), env::Fail> {
        self.pie
            .push(self.buf.as_ref().get(..self.len).unwrap_or(&[]))
            .map_err(|e| env::Fail::of_code(e.source.code()).unwrap_or(env::Fail::Denied))
    }

    /// 收一条报（**有界等**由参数说，**缓冲由调用方给**）。
    ///
    /// 失败三格**分得开**（[`Land`]）：[`Land::Expired`] = 没收到、[`Land::Unavailable`] =
    /// 这一枚孔用不动、[`Land::Unread`] = 收到了解不动。
    ///
    /// **照实记（为什么是分格失败，不是折成一个 `None`）**：门那两侧把失败都折成同一句 `BAD`
    /// （`Err(_) ⇒ BAD`，与从前那个 `None` 一字不差）；而**靠收帧结果判活**的循环要把它们分开
    /// ——供单那圈发货循环就是：没收到 ⇒ 去探对端还活着没有；收到了解不动 ⇒ 答一句 `BAD`。
    /// 两件事共用一个 `None` 是我上一版写的（照实记在那一刀里），今天由**类型**分开：
    /// 一件事一个名字，失败落在失败域上。
    ///
    /// **照实记（同词不同事）**：`Ledger::land` / `Operator::land` 是"落一格账 / 落一枚入口"，
    /// 这一手是"从孔里收一条报"。
    ///
    /// **照实记（为什么缓冲不是船台自己那只）**：发那一侧可以自己带——那条报是自己编的，超不出
    /// 本族最长。收这一侧不行：**推得进来什么，由载体定界**（一页），而 `M::Buf` 只保证"本族
    /// 合法的最长那一枚"。比 `Buf` 长、又在一页之内的那一条：核答 `Denied` 而**槽原样**（丢一条
    /// 消息不可逆）——拿 `Buf` 收就是读到一个"读不懂"，可那一枚**还留在槽里** ⇒ 组每轮都唤醒、
    /// 门每轮答一句 `BAD`，而这位客人下一次正经的推**堵在门外**（实测与修法见
    /// `harness/src/probe_bound.rs` 那两趟）。故凡"门"那一侧收帧都拿**载体那一页**来收：那一条
    /// 取得出来、解得失败 ⇒ 照旧答 `BAD`，槽也空了。客侧那一枚孔只有本族对面那一侧会推 ⇒
    /// 拿本族那只空缓冲（[`Message::EMPTY`]）就够。
    ///
    /// 表外的动作码**不是**任何一格失败，它是那一族 `In` 自己的一格（如 `Wire::Unknown`）。
    pub fn land(&self, buf: &mut [u8], millis: Wait) -> Result<M::In, Land> {
        let n = match self.pie.pull_timeout(buf, millis) {
            Ok(n) => n,
            // **收这一侧的失败也要分层**：内核答的码分成"再等等"与"别等了"两件事（见 [`Land`]）。
            // 表外的码按 [`Land::Expired`] 落——那是"没读到"里最不含承诺的一格（与
            // [`Slip::ship`] 把表外的码折成 `Denied` 同一条口径）。
            Err(e) => {
                return Err(match env::Fail::of_code(e.source.code()) {
                    Some(env::Fail::Dead | env::Fail::Denied) => Land::Unavailable,
                    _ => Land::Expired,
                });
            }
        };
        let frame = buf.get(..n).ok_or(Land::Unread)?;
        M::fetch(frame).ok_or(Land::Unread)
    }
}

/// 「收一条报」（[`Slip::land`]）那三格失败——**分得开**。
///
/// **照实记（为什么要分开）**：门那两侧只关心"这一问成没成"（怎么失败都答一句 `BAD`）；而
/// **判活**的循环要分开——"没收到"是去探对端还活着没有，"收到了解不动"是这一问自己的毛病。
/// 两件事两个下一步，故落成格。
///
/// **照实记（[`Land::Unavailable`] 是后加的，以及它为什么不是"对端没了"）**：原先只有两格，而
/// [`Slip::land`] 把 `pull_timeout` 的**任何**错误都折成 [`Land::Expired`]（`.map_err(|_| …)`）
/// ⇒ 内核那七枚词汇在收这一侧**一格都不剩**：`Dead`（这一枚孔已封印）与 `Busy`（期限内没等到）
/// 被说成同一件事。加这一格就是把它分层。
///
/// **它报的是"这个端点用不动了"，不是"对端没了"**——这两件事在这一层**本来就不同**：本端读的
/// 那一枚孔**命随本端**（对端退出时，内核那条寿命边封的是**对端开的那几枚**）⇒ "对端没了"在
/// 这条路上通常**报不出来**，那正是 `root` 发货循环要额外探活（`alive`）的理由（见
/// `programs/src/root/supply/server.rs` 的头注）。故那一处的 `Unavailable` 是"这一枚孔已经用
/// 不动"，与 `Expired` 的"再等等、顺手探一次活"分得开。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Land {
    /// **没收到**：期限内没等到（内核答 `Busy`，含表外的码）。
    Expired,
    /// **这个端点用不动了**（内核答 `Dead` / `Denied`）：这一枚孔已封印、或号本来就不对。
    /// 与 [`Land::Expired`] 是两件事——那是"再等等"，这是"别等了"。
    Unavailable,
    /// **收到了，解不动**：长度不对 / 形状不对（那是这一族 `fetch` 的判据）；
    /// 缓冲比帧还短也落这一格（那一格类型上到不了：缓冲按本族最长给）。
    Unread,
}
