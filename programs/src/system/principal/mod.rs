//! principal::实现侧 — **身份服务那一台**。
//!
//! 判据与 [`crate::system::operator`] 同款：**判定与接口**（正文、七条原语、帧、客侧那一面、
//! 两面各一枚 `Grant`）住 `crates/protocol/src/system/principal/`；**实现方**（**独立域**，
//! `prog-principal` 那一台）住这里。
//!
//! **比原计划少一个文件**：载体用的是 rtc 那一面已经量过的"**门牌自带回信孔**"，
//! 故**不需要**板/树那套提示孔 + 转授 + 客人账——`desk` 那个文件因此没有出现：
//! 一问一答的那一趟自带回信孔，"往哪回"不需要 Server 记住任何东西。
//!
//! **照实记（`bridge` 回来了）**：装配侧那两面（放行前 `bind`、名册起来之后 `adopt` 补绑）
//! 原先散在装配那一趟里；它们问的是名册的语义，故收进 [`bridge`] 那一间。
//!
//! **照实记（`mount` 那一份是开面那一刀来的，回炉那一刀又收了它）**：这一族从前**只有一枚
//! 门牌**（`/sys/principal` 那一格本身就是它），故"坐标与铸"只有两句、不必成文件。开面之后是
//! **两枚门牌 ＋ 两段末名**，"坐标与铸"因此成了文件（与另外三族同形）。而回炉那一刀量了一件事：
//! 四族那四份 `mount.rs` 里，"铸入口"那一手**逐字同构**（差族型与错误字面量）⇒ 它收进
//! [`crate::system::mount::entry`]；剩下的只有这条 `SEGMENT`（见下面那一格）。

//! **照实记（`core` 是残枝那一刀从 protocol 搬来的）**：名册与谱系那两张表原先住
//! `crates/protocol/src/system/principal/core.rs`——读者只有本域那一枚线程，故回这里。

use protocol::system::principal as pcall;

/// **那一段目录的名字**（`/sys/principal` 底下那一段，也即 `/sys/principal/{面名}` 的中间那一段）。
///
/// **它为什么住这里**（照实记：回炉那一刀把 `mount.rs` 整份收了）：那一段名字是**这一族自己的
/// 事实**，而"铸入口"那一手四族逐字同构、已收进 [`crate::system::mount::entry`]；一份文件只剩
/// 一条 `const` 就挣不来一个文件。名字的唯一来源在协议那一侧那一格（`pcall::NAME`），这里只引用。
pub const SEGMENT: &str = pcall::NAME;

pub mod bridge;
pub mod core;
pub mod server;
