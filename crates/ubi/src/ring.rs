//! ring — 一对一共享内存邮路（U→S 生态的 ABI 契约层）。
//!
//! ring = 一对一通道（open 即双端固定：两端各持一端，无 pier/quay 多对一计数）。
//! 数据面同样是用户态共享物理帧上的环形缓冲（与 dock 同布局），但**生命周期
//! 更简**：无 pier_count/quay 在场位/Hang——只有 Live→Dead 两态。
//!
//! 共享区布局（每 ring 一段连续物理帧，双端各自映射进本方 space）：
//!
//! ```text
//! offset 0x00  state:       AtomicU8   两态（Live / Dead，见内核 mail::ring）
//! offset 0x08  lock:        AtomicBool 用户态自旋锁（push/pull 临界区）
//! offset 0x10  read:        AtomicUsize 消费弧（消费端独占推进）
//! offset 0x18  write:       AtomicUsize 生产弧（生产端独占推进，锁内）
//! offset 0x20  item_len:    usize      定长项字节数（open 定型，只读）
//! offset 0x28  slots:       usize      槽数（2 的幂，open 定型，只读）
//! offset 0x30  buffer[..]               定长槽环
//! ```
//!
//! 与 dock 的差异（语义简化）：无 pier_count/quay/Hang——ring 两端身份固定，
//! 任一端 close 即对端感知断开（Dead）。槽定位同 dock（单调弧 + 2 的幂掩码）。
//!
//! 键面（见内核 envcall）：ring id 带最高位标记（[`RING_KEY_TAG`]，独立于
//! [`DOCK_KEY_TAG`]）即 ring 键，envcall Wait/Wake 见标记 → 不经
//! `WaitKey::compose`；否则照旧 compose 用户地址键。

/// ring 键标记位（id 的最高位，独立于 dock 的标记位——两个 id 空间不撞）。
pub const RING_KEY_TAG: usize = 1usize << 62;

// ── 共享区布局（编译期 ABI；与 dock 同源，偏移紧凑）──────────────

/// 状态槽偏移（AtomicU8；两态编码见内核 mail::ring 的 RingState）。
pub const OFF_STATE: usize = 0x00;
/// 自旋锁偏移（AtomicBool；1 = 持锁）。
pub const OFF_LOCK: usize = 0x08;
/// 读弧偏移（AtomicUsize）。
pub const OFF_READ: usize = 0x10;
/// 写弧偏移（AtomicUsize）。
pub const OFF_WRITE: usize = 0x18;
/// 项长（只读）。
pub const OFF_ITEM_LEN: usize = 0x20;
/// 槽数（只读，2 的幂）。
pub const OFF_SLOTS: usize = 0x28;
/// 缓冲起点。
pub const OFF_BUFFER: usize = 0x30;

/// 两态编码（与内核 `mail::ring::RingState` 判别式同值；两端同源）。
pub mod state {
    /// Live：两端在场。
    pub const LIVE: u8 = 0;
    /// Dead：显式 close / 对端离场。
    pub const DEAD: u8 = 1;
}

/// ring 错误码（与 dock/port 契约同源）。
pub mod err {
    /// 通道已断开（close / 对端离场）。
    pub const DEAD: isize = -1;
    /// 条件不满足（槽满/槽空）——wait 后重试，非终结态。
    pub const BUSY: isize = -2;
}
