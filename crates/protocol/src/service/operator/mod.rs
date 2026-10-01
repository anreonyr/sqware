//! 一棵树，名字 → Pie。
//! 全系统**就一个** Operator，管着所有条目；其他任务只是**操作**它：
//!    装配者（编排域 system）                    客人（某个服务域）        持树者（operator 域，一枚线程）
//!    endpoint(客人, LINK)                ▶   open: endpoint(生我者, LINK)
//!    转授：把客人那一枚 Ship 给持树者 ──────────────────────────────────▶  按"谁转授的 + 记号"认答话写端
//!    LINK 上先递一格：持树者的号 ────────────▶  open 收下 ⇒ 此后叫得出它
//!    提示：往提示之路推一个客人号 ─────────────────────────────────────▶  收一位客人（admit）
//!                                             铸问话孔(ask)、Ship 给持树者 ▶  按"谁开的 + 记号"认出 ⇒ arm + attach
//!                                             推(Req) ───────────────────▶  组唤醒 ⇒ pull ⇒ 交给树
//!                                             读(Said) ◀─────────────────  推(Union)
//!  ```
//!  # 为什么持树者就一枚线程
//!  树上那几枚是**一枚只在持它的那张表里有意义的句柄**（`PieToken` = "我这张表里的第几个"）。
//!  "查到了要把 Pie 授出去"必须由**持有那一枚的那张表**来做——故所有条目只能住同一张表，
//!  也就是同一枚线程。板那一台栽过这条（每位客人一枚待客线程 ⇒ 甲的条目在甲的表里，乙来查
//!  时判它"已死"、也授不出去，症状是"刚挂上的名字，别人一查就是 `Unknown`"）。

pub mod frame;

pub use frame::{EntryId, Fail, Where};

/// **树上的坐标**（Path：一条最多 Path::MAX 段的路）：装配者落格、客人译号、线上那一格，
/// 三处同一个形状（见 crate::common::path 头注那一张 std 对照表）。
pub use crate::common::path::Path;

pub mod client;

/// **操作面那一维**：一枚 `Grant` = 一枚操作（`part` / `land` / …）。
/// 它与「用」那一轴（Permit）与「改」那一轴（砖上的主人）**正交**：这一维只答"这一位
/// 许不许这一类"，判别落在**会话说的是哪一位**（会话入口的记号）上，故请求里没有可填的格。
pub mod grant;

pub use grant::Grant;

// operator 的**适配那一半** —— 内核那几只手的别名、立树、交出。
// （原 operator::call::X、今 operator::X）。**开会话那一手已抬进

/// **那一段目录的名字**（`/svc/sys/operator` 底下那一段，也即 `/svc/sys/operator/{面名}` 的中间那一段）。
pub const NAME: &str = "operator";

/// **本族那块窗格在树上的路**：`/svc/sys/operator`（头两段是四族共用的
/// crate::common::svc::DIR，末段是本族自己的名字 NAME）——**一处说全**。
pub const DIR: &Path = Path::new("svc/sys/operator");

pub use frame::{
    ASK_MARK, BAD, DENIED, FULL, LINK, Listing, NONEMPTY, OK, Req, Rule, Said, TIP_LEN, TIP_MARK,
    Tip, TipIn, UNJUDGED, UNKNOWN, Union, Wire, code_to_fail, fail_to_code,
};
pub use frame::{Permit, Ruling};

// 三格是**一组**，三个名字读成同一句式的被动式事实、故等长（9/9/9）：
