//! dock — 共享内存邮路（U→S 生态的 ABI 契约层）。
//!
//! dock = 方向性通道（多 pier 生产 / 唯一 quay 消费，词族 port 的对偶）；数据面
//! 是用户态共享物理帧上的环形缓冲，两端（内核 ring.rs 与用户 user::dock）都按
//! 本文件的布局与常量访问——布局是**编译期 ABI**，改它即改协议。
//!
//! 共享区布局（每 dock 一段连续物理帧，双端各自映射进本方 space）：
//!
//! ```text
//! offset 0x00  state:       AtomicU8   四态（Live/Hang/Gone/Dead，见内核 ring.rs）
//! offset 0x08  lock:        AtomicBool 用户态自旋锁（push/pull 临界区，统一并发正确性）
//! offset 0x10  pier_count:  AtomicUsize 现存 pier 端数（归零 → Hang）
//! offset 0x18  quay_present: AtomicBool quay 在场（join 独占；离场 → Dead）
//! offset 0x20  read:        AtomicUsize 消费弧（quay 独占推进）
//! offset 0x28  write:       AtomicUsize 生产弧（pier 共享推进，锁内）
//! offset 0x30  item_len:    usize      定长项字节数（open 定型，只读）
//! offset 0x38  slots:       usize      槽数（2 的幂，open 定型，只读）
//! offset 0x40  buffer[..]               定长槽环（槽 i 的项 = buffer + (i & (slots-1))*item_len）
//! ```
//!
//! 槽定位用单调弧计数 + 2 的幂掩码：`fold = write - read`（在途项数）；满 =
//! fold == slots、空 = fold == 0；槽位 = `(write & (slots-1))`。两弧只增不
//! 减（u64/usize 自然回绕在足够大的容量/消息量下不重叠，教学规模安全）。
//!
//! 键面（见内核 envcall）：dock id 带最高位标记（[`DOCK_KEY_TAG`]）即 dock 键，
//! envcall Wait/Wake 见标记 → 不经 `WaitKey::compose`（跨 team asid 不同，经
//! compose 必失配）；否则照旧 compose 用户地址键。

/// dock 键标记位（id 的最高位；dk 键 = `DOCK_KEY_TAG | id`）。
pub const DOCK_KEY_TAG: usize = 1usize << 63;

// ── 共享区布局（编译期 ABI）──────────────────────────────────

/// 状态槽偏移（AtomicU8；四态编码见内核 mail::ring 的 RingState）。
pub const OFF_STATE: usize = 0x00;
/// 自旋锁偏移（AtomicBool；1 = 持锁）。
pub const OFF_LOCK: usize = 0x08;
/// pier 计数偏移（AtomicUsize）。
pub const OFF_PIER_COUNT: usize = 0x10;
/// quay 在场偏移（AtomicBool）。
pub const OFF_QUAY: usize = 0x18;
/// 读弧偏移（AtomicUsize）。
pub const OFF_READ: usize = 0x20;
/// 写弧偏移（AtomicUsize）。
pub const OFF_WRITE: usize = 0x28;
/// 项长（只读）。
pub const OFF_ITEM_LEN: usize = 0x30;
/// 槽数（只读，2 的幂）。
pub const OFF_SLOTS: usize = 0x38;
/// 缓冲起点。
pub const OFF_BUFFER: usize = 0x40;

/// 四态编码（与内核 `mail::ring::RingState` 判别式同值；两端同源）。
pub mod state {
    /// Live：pier_count ≥ 1 且 quay 在场。
    pub const LIVE: u8 = 0;
    /// Hang：pier 全 drop——quay 仍可取余信。
    pub const HANG: u8 = 1;
    /// Gone：Hang 下余信取空（quay 钉连）→ 连接自然终了。
    pub const GONE: u8 = 2;
    /// Dead：显式 shut / quay 缺席。
    pub const DEAD: u8 = 3;
}

/// dock 错误码（D1 负码；与内核 mailbox 错误契约同源）。
pub mod err {
    /// 通道已断开（shut / quay 缺席 / 未登记）。
    pub const DEAD: isize = -1;
    /// 条件不满足（槽满/槽空/quay 被占）——wait 后重试，非终结态。
    pub const BUSY: isize = -2;
    /// Hang 下余信取空后钉连（Gone）——连接自然终了。
    pub const GONE: isize = -3;
}

/// 端选择（RingJoin/RingDrop 的 side 参数编码）。
pub mod side {
    /// pier（生产端）。
    pub const PIER: usize = 0;
    /// quay（消费端）。
    pub const QUAY: usize = 1;
}