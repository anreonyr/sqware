//! Operator Protocol — **命名寻址**：一棵树，名字 → Pie。
//! 全系统**就一个** Operator，管着所有条目；其他任务只是**操作**它：
//! ```text
//!   part  分   分出一块空窗格（Pane）
//!   land  落   把一枚 Pie 落到一个名字上（贴一枚 Tile）
//!   find  寻   走到头，把那一枚 Pie 交出去
//!   trim  剪   剪掉一条
//!   list  列   看一块 Pane 里有哪些**号**
//!   name  名   这枚号此刻叫什么

// **使用侧** [`client`]（客侧三手）住这里——那是"别的任务怎么找上树"。**今天的客人**：六台域
// （`canonical`（找控制台那两枚门牌 `/svc/drv/uart/{rx,tx}`）、`router` / `rtc` / `uart`（各把门牌挂上
// 树）、`principal` / `coalition`（上树那条 `/svc` 路））与测具一串（`harness` 的 `subject` /
// `member` / `guest` / `lodger` / `sleeper` / `probe_*`）——**装配者不在客人之列**：它替每一位客人
// 递孔，自己不上树（"往树上立一格"那件事由**持树者在自己核里落**，装配者只递那一枚与一条路，
// 见 [`Tip`] 与 `programs/src/system/operator/plate.rs::plate`）。
// **挂上树这件事合设计**：树是"名字 → 资源"那本目录，谁要挂谁自己上来（真客人是
// `harness/src/probe_control.rs`）。**实现侧**（持树者）
// 与**装配侧**（把持树者接上客人 / 认下提示之路）住 `programs/src/system/operator/{server,bridge}.rs`。
// 下面这段是那一台的说明——它讲的是"怎么跑"。
//!  同一手（[`endpoint`](crate::communication::establish::endpoint)：铸本端那一枚 ＋ 认下对端那一枚），
//!  靠**孔上的记号**对位。
//!  ```text
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

/// **树上的坐标**（一条最多 [`Path::MAX`] 段的路）：装配者落格、客人译号、线上那一格，
/// 三处同一个形状（见 [`path`] 头注那一张 std 对照表）。
pub mod path;

pub use path::Path;

pub mod client;

/// **操作面那一维**：一枚 `Grant` = 一枚操作（`part` / `land` / …）。
/// 它与「用」那一轴（[`Permit`]）与「改」那一轴（砖上的主人）**正交**：这一维只答"这一位
/// 许不许这一类"，判别落在**会话说的是哪一位**（会话入口的记号）上，故请求里没有可填的格。
pub mod grant;

pub use grant::Grant;

// 形与据就在本模块树下（`frame`）。

// operator 的**适配那一半** —— 内核那几只手的别名、立树、交出。
// 帧与码见 [`frame`]；本模块把那一整片**点名转出** ⇒ 调用点只在路径那一处改过
// （原 `operator::call::X`、今 `operator::X`）。**开会话那一手已抬进

/// **那一段目录的名字**（`/svc/sys/operator` 底下那一段，也即 `/svc/sys/operator/{面名}` 的中间那一段）。
pub const NAME: &str = "operator";

/// **本族那块窗格在树上的路**：`/svc/sys/operator`（头两段是四族共用的
/// [`crate::system::DIR`]，末段是本族自己的名字 [`NAME`]）——**一处说全**。
pub const DIR: &Path = Path::new("svc/sys/operator");

pub use frame::{
    ASK_MARK, BAD, DENIED, FULL, LINK, Listing, NONEMPTY, OK, Req, Rule, Said, TIP_LEN, TIP_MARK,
    Tip, TipIn, UNJUDGED, UNKNOWN, Union, Wire, code_to_fail, fail_to_code,
};
pub use frame::{Permit, Ruling};

// 三格是**一组**，三个名字读成同一句式的被动式事实、故等长（9/9/9）：
