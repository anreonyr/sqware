//! **一条报**——这一族会编会解的那条约定：缓冲多大、怎么写进去、怎么读回来。
//!
//! # 它替掉了什么
//!
//! 从前每一族各自散着一串**自由函数**：`pack_ask` / `unpack_ask` / `pack_reply` /
//! `unpack_reply` / `pack_list` / `read_list` / …，外加一串长度常量（`ASK_LEN` / `REPLY_LEN` /
//! `LIST_REPLY_LEN` / …）。同一件事的"多长、怎么写、怎么读"写三处，**改一处漏一处编得过**。
//!
//! 现在一族只有三样：
//!
//! ```text
//!   一张字段表（env::frame!）   偏移与长度一处求和得出
//!   一个 impl Message           这一族会编会解（变长那几枚在这里手写字节算术）
//!   用 Slip 收发                 protocol 那一层的端点
//! ```
//!
//! # 为什么缓冲的尺寸是**关联类型**、不是关联常量
//!
//! `Slip<M>` 要把缓冲**装在自己身上**（"编好，躺在船台自己那只缓冲里"）。关联常量那条路
//! （`buf: [u8; M::MAX]`）在本地 nightly 上实测：**4 处** `unconstrained generic constant`，
//! 补 `where [(); M::MAX]:` 之后能编过，但那条尾巴**渗到凡是提到 `Slip<M>` 的地方**（连
//! "只把它当字段"的类型也要），另需 `#![allow(incomplete_features)]`，且开这个门的 crate
//! **当场退出 next-gen trait solver**。改成关联类型（`type Buf = [u8; 41]`）之后：**0 错、
//! 0 门、0 尾巴**——同一条"尺寸一处定义"，一笔债都不欠。
//!
//! `EMPTY` 那一格是因为 `[u8; 41]` **没有 `Default`**（Rust 的数组 `Default` 只到 32），
//! 故空缓冲由族给一枚。

/// **一条报**：一族会编会解的那条约定。
///
/// `In` 是**解开之后的形状**（板上是 `Wire`）——与报文类型成对，故一个 `impl` 同时给出发与收。
pub trait Message {
    /// 解开之后的形状。
    type In;

    /// 这一族的缓冲。尺寸就是这一族最长那一枚报文（一处定义）。
    type Buf: AsRef<[u8]> + AsMut<[u8]> + Copy;

    /// 空缓冲（`[u8; 41]` 没有 `Default`，故由族给）。
    const EMPTY: Self::Buf;

    /// 这一族最长那一枚占几字节——**由 `Buf` 得出**，不必各族再写一遍。
    const MAX: usize = core::mem::size_of::<Self::Buf>();

    /// 编进 `out`，返**实际长度**（形状不同则长度不同）。
    ///
    /// `None` = 缓冲不够（`Buf` 就是 `MAX`，故这一支只在类型被写错时才到得了——不 `panic`）。
    fn store(&self, out: &mut [u8]) -> Option<usize>;

    /// 从 `bytes` 读回来。**读不懂 ⇒ `None`**（不猜、不崩）。
    ///
    /// "长度为该形状该有的长度"是帧的契约，故**短一字节即读不懂**；长一字节算不算读得懂，
    /// 由各族自己判（板那一族判"不算"，见 `system::board::frame`）。
    fn fetch(bytes: &[u8]) -> Option<Self::In>;
}
