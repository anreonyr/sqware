//! coalition::实现侧 — **结盟服务那一台**。
//!
//! 判据与 [`crate::system::principal`] 同款：**判定与接口**（正文、六条原语、帧、客侧
//! 那一面）住 `crates/protocol/src/system/coalition/`；**实现方**（**独立域**，`prog-coalition`
//! 那一台）住这里。
//!
//! **与本目录里另两位一样少文件**：载体用的是 rtc 那一面量过的"**门牌自带回信孔**"，
//! 故**不需要**提示孔 + 转授 + 客人账——`bridge` 与 `desk` 两个文件因此没有出现。
//!
//! **它与 principal 那一台的差别只有一处**：它多了一个**客人身份**——起手在树上找到
//! `/sys/principal/ask`（**只要问面**：本域只 `Resolve`），每条**写**原语嵌一次
//! `Resolve(发送者)`。"self"那一格因此不在核心，在这一层（正文"已知边界"里写着这一条的确切含义）。

//! **照实记（`core` 是残枝那一刀从 protocol 搬来的）**：盟册那本账原先住
//! `crates/protocol/src/system/coalition/core.rs`——读者只有本域那一枚线程，故回这里；
//! 协议那一边只剩号 / 失败域 / 一窗号。

use protocol::system::coalition as ccall;

/// **那一段目录的名字**（`/sys/coalition` 底下那一段，也即 `/sys/coalition/{面名}` 的中间那一段）。
///
/// **它为什么住这里**（照实记：回炉那一刀把 `mount.rs` 整份收了）：那一段名字是**这一族自己的
/// 事实**，而"铸入口"那一手四族逐字同构、已收进 [`crate::system::mount::entry`]；一份文件只剩
/// 一条 `const` 就挣不来一个文件。名字的唯一来源在协议那一侧那一格（`ccall::NAME`），这里只引用。
pub const SEGMENT: &str = ccall::NAME;

pub mod core;
// **照实记（`mount` 那一份是开面那一刀来的，回炉那一刀又收了它）**：这一族从前**只有一枚
// 门牌**（`/sys/coalition` 那一格本身就是它），故"坐标与铸"只有两句、不必成文件。开面之后是
// **两枚门牌 ＋ 两段末名**，"坐标与铸"因此成了文件（与另外三族同形）。而回炉那一刀量了一件事：
// 四族那四份 `mount.rs` 里，"铸入口"那一手**逐字同构** ⇒ 它收进
// [`crate::system::mount::entry`]；剩下的只有这条 `SEGMENT`（见上面那一格）。
pub mod server;

