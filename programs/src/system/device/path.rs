//! 这一景的产品 DMA 走哪一条路。

/// 这一景的产品 DMA 走哪一条路。
///
/// 它是**装配**的事实（由景名给出），不是运行时开关：普通产品是 [`Path::Normal`]，
/// 生产里没有一条常开的故障后门。两个起手故障点都是**受控注入**（就在装配序列里
/// `say` 一句再 `Err`），与“硬件没在期限内回答”那条自然超时**不是同一件事**
/// ——后者走 `driver::iommu::handshake::Handshake` 到期，报的是 `stop pending`。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Path {
    /// 普通产品：显式 `Close::Normal` 收口，起手不注入。
    Normal,
    /// 故障景：收服务走 `offline(Fault)`，且平台第一次停住答 `Pending`。
    Fault,
    /// 起手受控失败：`spawn` 之后、**发布之前**（范围已登记，可达标记还没点）。
    StartupBeforePublish,
    /// 起手受控失败：**发布与武装都过了**、交付之前。
    StartupAfterPublish,
}
