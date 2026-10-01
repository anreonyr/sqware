//! rtc::adapt — **住持面（适配）**：碰内核、碰板、碰设备的那一半。
//! ```text
//!   desk.rs      门面：解帧 → 认孔 → 喂会话核 → 执行它吐的答形
//!   resident.rs  常驻：一只组等两个源，喂事件、执行动作（**壳**）
//! ```

pub mod desk;
pub mod resident;
