//! :实现侧 — 身份服务那一台。
//! 两面各一枚 `Grant`）住 `crates/protocol/src/service/principal/`；**实现方**（**独立域**，
//! 故**不需要**板/树那套提示孔 + 转授 + 客人账——`desk` 那个文件因此没有出现：

/// 核（纯）：名册 core::roster ＋ 谱系 core::lineage，立账与跨两张表那一手（`adopt`）
/// 在 core 本身
pub mod bridge;
pub mod core;
pub mod serve;
