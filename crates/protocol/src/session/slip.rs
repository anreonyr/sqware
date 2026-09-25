//! **船台** —— 一条泊位上的端点：一枚孔 ＋ **这条路上流的那一种报**（类型绑定）。
//!
//! ```text
//!   Slip::seal(pie)                 认下一枚孔 ⇒ 一个船台（类型在这儿绑上）
//!         .load(m)                  装上一条报（编进船台自己那只缓冲）
//!         .ship()                   发出去
//!
//!   slip.land(buf, millis)          收一条（缓冲由调用方给；读不懂 / 超时 ⇒ None）
//! ```
//!
//! # 为什么它是一层库
//!
//! 四个方法只碰**两枚原语**（`HolePie::push` / `pull_timeout`）与**一条约定**
//! （[`Message`]），与哪一族、哪条路、什么荷载全无关。它住 `protocol`
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

    /// 装上一条报（编进自己那只缓冲）。
    ///
    /// `Message::store` 返 `None`（缓冲不够）时长度按 0 记——`Buf` 就是这一族的最长，故那一支
    /// 只在类型被写错时才到得了（`store` 的文档记着）。
    pub fn load(mut self, m: M) -> Self {
        self.len = m.store(self.buf.as_mut()).unwrap_or(0);
        self
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

    /// 收一条（**有界等**由参数说，**缓冲由调用方给**）。
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
    /// 返 `None` 盖两件事：**期限到了还没到**与**读不懂**——板的持板者正是这么用的
    /// （`None` ⇒ 答 `BAD`）。表外的动作码**不是** `None`，它是那一族 `In` 自己的一格
    /// （如 `Wire::Unknown`）。
    pub fn land(&self, buf: &mut [u8], millis: Wait) -> Option<M::In> {
        let n = self.pie.pull_timeout(buf, millis).ok()?;
        M::fetch(buf.get(..n)?)
    }
}
