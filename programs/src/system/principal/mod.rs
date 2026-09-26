//! principal::实现侧 — **身份服务那一台**。
//!
//! 判据与 [`crate::system::operator`] 同款：**判定与接口**（正文、九条原语、帧、客侧那一面）
//! 住 `crates/protocol/src/system/principal/`；**实现方**（**独立域**，`prog-principal` 那一台）
//! 住这里。
//!
//! **比原计划少一个文件**：载体用的是 rtc 那一面已经量过的"**门牌自带回信孔**"，
//! 故**不需要**板/树那套提示孔 + 转授 + 客人账——`desk` 那个文件因此没有出现：
//! 一问一答的那一趟自带回信孔，"往哪回"不需要 Server 记住任何东西。
//!
//! **照实记（`bridge` 回来了）**：装配侧那两面（放行前 `bind`、名册起来之后 `adopt` 补绑）
//! 原先散在 `System::bring_up` 里；它们问的是名册的语义，故收进 [`bridge`] 那一间。

pub mod bridge;
pub mod server;
