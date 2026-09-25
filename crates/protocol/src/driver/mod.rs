//! driver — **正文已搬进「约」**（`crates/contract/src/driver/mod.rs`）。
//!
//! 这里只剩**碰内核的那几件**（客侧那几手、十件手的身体、绑真手的构造与两张对照表）——
//! 判据见 `crates/protocol/src/lib.rs` 与 `crates/contract/src/lib.rs`。


pub mod line;
pub mod supply;

/// 驱动族在命名树上的那一段目录：**`/device`**。
///
/// 驱动把自己的**服务入口**落在 `/device/<服务名>` 上（`router` ⇒ `/device/router`），名字用
/// **服务名**——与装配单、日志、板上的名字同一个。
///
/// 那块 Pane 归**第一个上树的驱动**建：树上 `part` 落到一块非空 Pane 上答 `NonEmpty`
/// ⇒ 只有一次创建机会，故"已经在了"必须当成**要的结果**（不是错误）。
pub const DIR: &str = "device";
