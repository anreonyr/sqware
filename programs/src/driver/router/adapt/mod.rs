//! router::adapt — **住持面（适配）**：碰内核、碰板、碰树、碰设备的那一半。
//! ```text
//!   boot.rs      起手：领配给 → 开两图 → 读树 → 建账 → 铸入口 → 上板 ＋ 上树 → 挂组
//!   event/       三个源各一手：
//!     desk.rs    门上有人：登记（解帧 → 解树 → 占格 → 接线 → 答）
//!     bell.rs    铃：领一条 → 投一帧 → 投到了才静音 ＋ 结 → 报过没有那一行
//!     exhaust.rs 排空：客人说"我排空了"——那一格回闲 + 把线放回去
//!   sweep.rs     逐客：主人没了的那些线——拆线 + 空出格子
//!   resident.rs  常驻**壳**：等三源 → 四手各就位
//! ```

pub mod boot;
pub mod event;
pub mod resident;
pub mod sweep;
