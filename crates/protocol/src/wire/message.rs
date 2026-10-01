//! # 它替掉了什么

/// **一条报**：一族会编会解的那条约定。
/// `In` 是**解开之后的形状**（板上是 `Wire`）——与报文类型成对，故一个 `impl` 同时给出发与收。
pub trait Message {
    /// 解开之后的形状。
    type In;

    /// **发的那只在 Sender::send 的栈帧上借一只**（拿 EMPTY 起）；**收的那只
    type Buf: AsRef<[u8]> + AsMut<[u8]>;

    /// 空缓冲（`[u8; 41]` 没有 `Default`，故由族给）。**收帧的调用方也拿它开一只**：客侧那一枚
    /// 孔只有本族对面会推，本族最长那么大就够。
    const EMPTY: Self::Buf;

    const MAX: usize = core::mem::size_of::<Self::Buf>();

    /// 编进 `out`，返**实际长度**（形状不同则长度不同）。
    /// `None` = 缓冲不够（`Buf` 就是 `MAX`，故这一支只在类型被写错时才到得了——不 `panic`）。
    fn store(&self, out: &mut [u8]) -> Option<usize>;

    /// 从 `bytes` 读回来。**读不懂 ⇒ `None`**（不猜、不崩）。
    /// "长度为该形状该有的长度"是帧的契约，故**短一字节即读不懂**；长一字节算不算读得懂，
    /// 由各族自己判（各族的帧自己说）。**一格自己的值合不合规矩**
    /// 也归它的 env::wire::Field（如今 `bool` 那一格**只认 0 / 1**，畸形的 `2` 整个读不懂）。
    fn fetch(bytes: &[u8]) -> Option<Self::In>;
}
