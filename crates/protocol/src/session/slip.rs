//! **船台** —— 一条泊位上的端点：一枚孔 ＋ **这条路上流的那一种报**（类型绑定）。
//!
//! ```text
//!   Slip::seal(pie)                 认下一枚孔 ⇒ 一个船台（类型在这儿绑上）
//!         .load(m)                  装上一条报（编进自己那只缓冲）
//!         .ship()                   发出去
//!
//!   slip.land(millis)               收一条（读不懂 / 超时 ⇒ None）
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
    /// **照实记（返回的为什么是 `EnvResult` 而不是 `env::Fail`）**：裁的那一版写的是
    /// `Result<(), env::Fail>`，而**孔那一手原样报的是 `EnvResult`**（`erra::Error<EnvError>`）
    /// ——仓里今天**没有**那道读法（`Fail` 是**词汇**，`EnvError` 是**对线的读法**）。硬映射成
    /// 某一格（比如 `Denied`）会把 `Busy`（槽满）与 `Dead`（孔没了）都说成"没权限"，那是假话。
    /// 故这一手把原样的错误转出去，各族按自己那张负码表收（今天各处就是这么做的：
    /// `.map_err(|_| Fail::…)`）。要收成 `env::Fail`，得先在 `env` 立一道"码 → 词汇"的读法。
    pub fn ship(self) -> env::EnvResult<()> {
        self.pie.push(self.buf.as_ref().get(..self.len).unwrap_or(&[]))
    }

    /// 收一条（**有界等**由参数说）。
    ///
    /// **照实记（同词不同事）**：`Ledger::land` / `Operator::land` 是"落一格账 / 落一枚入口"，
    /// 这一手是"从孔里收一条报"。
    ///
    /// 返 `None` 盖两件事：**期限到了还没到**与**读不懂**——板的持板者正是这么用的
    /// （`None` ⇒ 答 `BAD`）。表外的动作码**不是** `None`，它是那一族 `In` 自己的一格
    /// （如 `Wire::Unknown`）。
    pub fn land(&self, millis: Wait) -> Option<M::In> {
        // `Buf: Copy` ⇒ 借一份出来收（`&self` 不动自己那只）。
        let mut buf = self.buf;
        let n = self.pie.pull_timeout(buf.as_mut(), millis).ok()?;
        M::fetch(buf.as_ref().get(..n)?)
    }
}
