//! coalition::实现侧 — **结盟服务那一台**。
//!
//! 判据与 [`crate::supervisor::principal`] 同款：**判定与接口**（正文、六条原语、帧、客侧
//! 那一面）住 `crates/protocol/src/coalition/`；**实现方**（iii 之后是**编排域里的一枚线程**，`Role::League`）住这里。
//!
//! **照实记（与本目录里另两位一样少文件）**：载体用的是 rtc 那一面量过的"**门牌自带回信孔**"，
//! 故**不需要**提示孔 + 转授 + 客人账——`bridge` 与 `desk` 两个文件因此没有出现。
//!
//! **它与 principal 那一台的差别只有一处**：它多了一个**客人身份**——起手在树上找到
//! `/sys/principal`，每条**写**原语嵌一次 `Resolve(发送者)`。"self"那一格因此不在核心，
//! 在这一层（正文"已知边界"里写着这一条的确切含义）。

pub mod fail;
pub mod server;
