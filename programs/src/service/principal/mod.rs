//! principal::实现侧 — **身份服务那一台**。
//! 判据与 [`crate::service::operator`] 同款：**判定与接口**（正文、七条原语、帧、客侧那一面、
//! 两面各一枚 `Grant`）住 `crates/protocol/src/service/principal/`；**实现方**（**独立域**，
//! `prog-principal` 那一台）住这里。
//! **比原计划少一个文件**：载体用的是 rtc 那一面已经量过的"**门牌自带回信孔**"，
//! 故**不需要**板/树那套提示孔 + 转授 + 客人账——`desk` 那个文件因此没有出现：
//! 一问一答的那一趟自带回信孔，"往哪回"不需要 Server 记住任何东西。

/// 核（纯）：名册 [`core::roster`] ＋ 谱系 [`core::lineage`]，立账与跨两张表那一手（`adopt`）
/// 在 [`core`] 本身。
pub mod bridge;
pub mod core;
/// 那一枚线程（服务侧 / 持树侧）与它叫的那几手住这里。
pub mod serve;
