//! principal::实现侧 — **身份服务那一台**。
//!
//! 判据与 [`crate::supervisor::system::operator`] 同款：**判定与接口**（正文、九条原语、帧、客侧那一面）
//! 住 `crates/protocol/src/principal/`；**实现方**（iii 之后是**编排域里的一枚线程**，`Role::Roster`）
//! 住这里。
//!
//! **照实记（比原计划少两个文件）**：载体用的是 rtc 那一面已经量过的"**门牌自带回信孔**"，
//! 故**不需要**板/树那套提示孔 + 转授 + 客人账——`bridge` 与 `desk` 两个文件因此没有出现：
//! 一问一答的那一趟自带回信孔，"往哪回"不需要 Server 记住任何东西。

pub mod fail;
pub mod server;
