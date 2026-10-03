//! 一棵树，名字 → Pie。
//!
//! 所有条目**只**在这一张表里；其他任务只是操作这张表（按名建条目、循名查 Pie、删条目）。
//!
//! # 树上的角色
//!
//! 装配者（system 域）按名建条目；客人（别的服务域）按名查 Pie；持树者（operator 域，一枚线程）
//! 是这张表的权威——只有它能造 Pie / 删条目。其他人要"动这一棵树"都把持树者当通道。
//!
//! # 为什么持树者只一枚线程
//!
//! 树上每枚 Pie 是"只在这一张表里有意义"的句柄（`PieToken` = 这张表里的第几号）。
//! "查到了要把 Pie 授出去"必须由**持有这张表**的那枚线程做——所以所有条目只能住同一张表，
//! 也就是同一枚线程。

pub mod frame;

pub use frame::{EntryId, Fail, Where};

/// 树上的坐标：一条最多 [`Path::MAX`] 段的路。
/// 装配者按它落格、客人按它译号、线上那一格就是它自己。
pub use crate::common::path::Path;

pub mod client;

/// **操作面那一维**：一枚 `Grant` = 一枚操作（`part` / `land` / …）。
///
/// 与「用」（Permit、与「改」（砖上的主人）**正交**：这一维只答"这一位许不许这一类"。
/// 判别落在**会话说的是哪一位**（会话入口的记号）上，故请求里没有可填的格。
pub mod grant;

pub use grant::Grant;

// operator 的**适配那一半** —— 内核那几只手的别名、立树、交出。
// （原 operator::call::X、今 operator::X）。**开会话那一手已抬进 ...**

/// 本族那块窗格在树上的中间段名字（`/svc/sys/operator/{面名}` 的中间那一段）。
pub const NAME: &str = "operator";

/// 本族那块窗格在树上的路：`/svc/sys/operator`。
pub const DIR: &Path = Path::new("svc/sys/operator");

pub use frame::{
    ASK_MARK, BAD, DENIED, FULL, LINK, Listing, OK, Req, Said, TIP_LEN, TIP_MARK, Tip, TipIn,
    UNJUDGED, UNKNOWN, Union, Wire, TIP_BACK, code_to_fail, fail_to_code,
};
pub use frame::{Permit, Ruling};

// 三格是**一组**，三个名字读成同一句式的被动式事实、故等长（9/9/9）：
