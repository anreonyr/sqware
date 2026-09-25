//! assembly — **装配单的词汇**：一行程序（[`Program`]）由哪些格子组成。
//!
//! # 为什么它住 `env`
//!
//! **同一张表要两侧读**：内核的 `build.rs`（**宿主**）按它决定"哪几台进哪张镜像"，编排域
//! （**riscv**）按它起程序。而仓里**只有 `env` 两边都编得过**——`protocol` 拖着 `runtime`
//! （那两处 riscv 内联汇编在宿主上编不过，`protocol-case` 那台宿主靶就是为此存在的——它如今
//! 走「约」`crates/contract` 这条依赖边，不再复制模块树）。
//!
//! 故这一层是"**两边都要知道的东西**"的定义处，与 [`env::fid`]（调用号）、
//! [`crate::manifest`]（清单格式）、[`env::Permission`] / [`env::Access`] /
//! [`env::Policy`] / [`env::ProgramKind`] 同款。
//!
//! **照实记（这些词是从别处搬下来的，旧路径照旧）**：`Announce` 原住
//! `protocol::system::desk`、`Grant` 原住 `programs::system::server`、
//! `Died` 原住 `programs::service`——三处现在都是 `pub use` 转发，**调用点一行没改**
//! （与 `Access`/`Policy` 从 `runtime::core::port` 搬到 `env::permission` 是同一条先例）。

use crate::key::Key;
use crate::supply::{Kind, Need, class_block};
use env::{Access, Policy};
use env::{Permission, PieToken, ProgramKind};

/// **怎么知道它起来了**——每个 Service 自己的一种，**登记时定死**。
///
/// 这一格不能一刀切：有的服务起来时会交回一条通道（那枚句柄的到达就是它的"我好了"），
/// 有的**什么都不交**（比如只走调试面的回显——它没有通道可交）。它是这一行的属性，
/// 不是调用 `start` 时的一个开关，故与名字、身子、状态同住一行。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Announce {
    /// 它会交回一枚句柄 ⇒ 那枚到了才算起来。
    Channel,
    /// 它不宣布 ⇒ **放行即起来**（"起来了"= 它没死）。
    None,
}

/// 起跑前要交出去的一枚门闩：给哪一枚、多大权。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Grant {
    /// 要交出去的那一枚（**在我表里**的句柄）。
    pub token: PieToken,
    /// 交出去的权限子集。
    pub perm: Permission,
}

/// 装配失败的编号：指"死在装配的哪一步"（沿用旧树那套小整数编号的意思）。
///
/// [`env::Reason`] 的别名——装配编号只是"域自己的小整数"那一族（见 [`env::exit`] 的头注），
/// 不另立类型；名字留着是因为装配单这张表通篇讲的是"哪一台、死在第几步"。
pub type Died = env::Reason;

// ── 死在装配的哪一步（编号沿用旧树那套小整数）───────────────────────────────
//
// **照实记（iii 之后 16 个；`probe-bound` 那一台上表再添一枚 ⇒ 今天 17 个）**：这批编号原住
// `programs/src/system/main.rs`，与装配单同源（"哪一台、死在第几步"），故随表一起搬下来；
// 那里现在 `pub use` 转发。iii 把**内件那三枚**随它们那三行送去了
// `programs/src/system/inner.rs`（`E_TREE` / `E_PRINCIPAL` / `E_COALITION` = 10 / 14 / 16，
// 由那一处自己持有）。故下面这几格里有空号（2..4 / 10 / 14 / 16）——**那是旧树的号**
// （见本节标题），不重排。
pub const E_BOOT: Died = 1;
pub const E_ROUTER: Died = 5;
pub const E_ECHO: Died = 6;
pub const E_GUEST: Died = 7;
pub const E_PASSER: Died = 8;
pub const E_UART: Died = 9;
pub const E_LODGER: Died = 11;
pub const E_RTC: Died = 12;
pub const E_SLEEPER: Died = 13;
pub const E_SUBJECT: Died = 15;
pub const E_MEMBER: Died = 17;
pub const E_PROBE: Died = 18;
pub const E_PROBE_OWNER: Died = 19;
pub const E_PROBE_LEASE: Died = 20;
pub const E_PROBE_RULE: Died = 21;
pub const E_PROBE_OTHER: Died = 22;
pub const E_PROBE_BOUND: Died = 23;

// ── 四张硬件需求单（**收方开的**，逐字从各域的 `needs.rs` 搬下来）────────────
//
// **照实记（为什么住这里）**：它们本来住在各自的域里（"它是**收方**开的那张单子"），而装配单
// 要把它们摆出来（`Plan::needs`）⇒ 必须与装配单同层。各域的 `needs.rs` 现在是 `pub use` 转发，
// **调用点一行没改**，那句"它住在本域里"仍由那一处读得出来。

/// 线路由者要的那三样：**中断控制器**（按类要）+ **设备树本体 / 门铃**（boot 造的，按已知坐标）。
pub const ROUTER_WANTS: &[Need] = &[
    Need::class(
        class_block("sifive,plic-1.0.0"),
        Kind::Pole,
        Access::FETCH_STORE,
        Policy::ONLY,
    ),
    Need::known(Key::dtb(), Kind::Pole, Access::FETCH, Policy::NONE),
    Need::known(Key::irq(), Kind::Nole, Access::FETCH, Policy::NONE),
];

/// 串口驱动要的那一枚：**那一台 `ns16550a`**。
pub const UART_WANTS: &[Need] = &[Need::class(
    class_block("ns16550a"),
    Kind::Pole,
    Access::FETCH_STORE,
    Policy::ONLY,
)];

/// 实时钟驱动要的那一枚：**那一台 `google,goldfish-rtc`**。
pub const RTC_WANTS: &[Need] = &[Need::class(
    class_block("google,goldfish-rtc"),
    Kind::Pole,
    Access::FETCH_STORE,
    Policy::ONLY,
)];

/// 房客要的那一枚：**一条没人要的线**（`virtio,mmio`），领上就死。
pub const LODGER_WANTS: &[Need] = &[Need::class(
    class_block("virtio,mmio"),
    Kind::Pole,
    Access::FETCH,
    Policy::ONLY,
)];

/// **这一台是什么**——**角色**，与"进哪张镜像"（[`Row::scenes`]）分开的一格。
///
/// **照实记（这一格原先三个变体，其中一个叫 `Product` —— 名不副实）**：那时它是"内核起的那套
/// 服务 + 真客人"一档，注释还写着 **"去掉它，机器不成机器"**。加 `product` 那一景**当场把这句话
/// 证伪了**：六位常客（`guest` / `passer` / `lodger` / `sleeper` / `subject` / `member`）**全去掉**，
/// 机器照起照停（实测：9 条镜像、六沓用例 22 例全绿、停机行在）——它们是**量服务的**，不是机器的
/// 骨头。而"变体名与景名同名"正是"两处说同一件事"的祸根：`guest` 那一族写着 `Product`，却不进
/// 产品镜像。故按**真实角色**重分，且**名字一律不用景名**（谁进哪张镜像只有 `scenes` 一处说）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Spot {
    /// 两个**域**：引导域（`root`）与编排域（`system`）——机器本身的骨架，都由内核那 8 字节前言
    /// 指到的那一条派生（见 [`ENTRY`]）。
    Domain,
    /// **常驻服务**：三台驱动（`router` / `uart` / `rtc`）——用户裁定"真正要发出去的那一台"
    /// 装的就是这几台。
    ///
    /// **照实记（原先还列着三个名字）**：持树者（`operator`）· 身份（`principal`）· 结盟
    /// （`coalition`）原先也是这一档。iii 之后它们**住编排域自己的域里**（`scenario.rs` 的
    /// `INNER`），不再是镜像里的程序——"装配单里有什么"与"编排域起什么"从此不重合，
    /// 而后者那三行由**编排域自己**持有。
    Service,
    /// **调试回显**（`echo`）：只走 `env` 调试面的那一条（U 态）——产品镜像里它排**最后一条**，
    /// 编排域等它退场才收场。
    Console,
    /// **常客**：产品侧的客人（**不是探针**）——量服务用的；去掉它，机器照转。
    Guest,
    /// **只读数**的探针：负证那一族，量的是门禁答得对不对。
    Probe,
    /// 压测台与它们的受害者：**整台替换引导镜像**，与验收场景没有交集。
    Rig,
}

/// **这一台是持树者的哪一双眼睛**——协调那一帧（`operator::bridge` 的 `COORD`）的后 8 字节
/// 就用它。
///
/// 它是**装配单上的一格**，不是靠名字认的：旧法写 `p.name == "principal"`——装配单上把那一行
/// 改个名，认它的那一侧就**静默失灵**（门禁从此判不了身份）。与 [`Plan::holds_tree`] 同一形状。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Eyes {
    /// 名册（`/sys/principal`）：答"这一位此刻代表谁"与"在不在他那一支里"。
    Roster = 0,
    /// 盟册（`/sys/coalition`）：答"这一位在那枚盟里吗"。
    League = 1,
}

impl Eyes {
    /// 线上那一格 → 这一枚（`None` = 表外，读不懂）。两侧共用同一份定义，故不必各写一遍常量。
    pub const fn of_wire(raw: u64) -> Option<Eyes> {
        match raw {
            x if x == Eyes::Roster as u64 => Some(Eyes::Roster),
            x if x == Eyes::League as u64 => Some(Eyes::League),
            _ => None,
        }
    }
}

/// **编排域起它时要的那些格**——`None` = 不经装配单起（引导域 / 编排域自己 / 压测台）。
#[derive(Clone, Copy)]
pub struct Plan {
    /// 默认那一景的**起手位次**。
    pub order: u8,
    pub announce: Announce,
    pub tokens: &'static [Grant],
    pub channels: &'static [&'static str],
    pub needs: Option<&'static [Need]>,
    pub board: bool,
    pub operator: bool,
    pub bind: bool,
    pub holds_tree: bool,
    /// **它是哪一双眼睛**（`None` = 不是：绝大多数行都不是）。
    pub eyes: Option<Eyes>,
    pub died: Died,
}

/// **这一景由哪一条起**（景名 → 程序名）：打包时"清单里第几条当引导镜像"（`root_at`）查的就是它。
///
/// **照实记（"景名 = 引导镜像名"那条巧合断了）**：从前每一景的引导镜像都跟景同名——`root` 那一景
/// 起 `root`、`rig` 起 `rig`……故打包那一格直接拿景名当程序名去找。加 `product` 那一景时它当场
/// 红了：`initrd: SQWARE_ROOT=product 不在这一景的清单里`。产品镜像的引导镜像**仍是 `root`**
/// ——同一个引导域起两景，正是"真正要发出去的那一台"与"验收镜像"该有的关系（用户原话：
/// "以后真正的程序放哪里"）。故这一格显式写出来，不再从景名推。
///
/// **它也是"有哪些景"的唯一一览**：一个景存在 ⇔ 它有一条引导镜像（打包那一侧按它报"认得的景"）。
pub const ENTRY: &[(&str, &str)] = &[
    ("root", "root"),
    ("product", "root"),
    ("rig", "rig"),
    ("load", "load"),
    ("group", "group"),
    ("beat", "beat"),
    ("again", "again"),
];

/// 这一景的引导镜像（`None` = 没这个景）。
pub fn entry_of(scene: &str) -> Option<&'static str> {
    ENTRY.iter().find(|(s, _)| *s == scene).map(|(_, e)| *e)
}

/// **装配单的一行**——加一台程序就写这一行（外加 cargo 的 `[[bin]]`，那是 cargo 的要求）。
#[derive(Clone, Copy)]
pub struct Row {
    /// 清单名（也是 cargo 的 bin 名去掉 `prog-`，见 `crates/image` 那条照实记）。
    pub name: &'static str,
    /// 装成哪种空间。
    pub kind: ProgramKind,
    /// 这一台是什么：**角色**。"进哪张镜像"是下一格 [`Row::scenes`]——两者**不重合**（常客也在
    /// 产品侧，却不进产品镜像），故不许拿这一格当"装不装"用。
    pub spot: Spot,
    /// 进哪几张引导镜像（**景名**）——**次序即装载次序**。
    pub scenes: &'static [&'static str],
    /// 装配参数；`None` = 不由编排域起。
    pub plan: Option<Plan>,
}

/// **装配单**：镜像里可能有的全部程序。
///
/// **次序是硬事实**：它就是装载次序（`ROOT_OFFSET` 按位次算），且各景按它过滤 ⇒ 加一行要
/// 想清楚放哪。**照实记（为什么这一张表能替掉三处）**：它同时答四个问题——装成哪种空间
/// （`kind`）· 是什么（`spot`）· 进哪几张镜像（`scenes`）· 编排域怎么起它（`plan`）——
/// 而这些原先散在 `kernel/build.rs` 的三张表与 `scenario.rs` 的 19 个 `const fn` 里。
///
/// **照实记（`product` 那一景是用户裁定的"真正要发出去的那一台"）**：`root` 那一景是**验收镜像**
/// ——表里每一条都装上（探针与试客都在里面，故那一景读数最全）。而"这台机器真正要发出去的样子"
/// 是另一景：**3 台服务 + `echo`**（外加两个域 `root` / `system`，镜像共 **6** 条）。
///
/// **照实记（iii：那一景从 9 条缩到 6 条）**：持树者 / 身份 / 结盟原先也各是一条，现在它们住
/// **编排域自己的域**里（`scenario.rs` 的 `INNER`）——不在镜像里，故不占条数。`echo` 排在最后，编排域
/// 等它退场——读到一行 `exit` 才收场（`scenario.rs` 那条照实记）。六位常客（`guest` / `passer` /
/// `lodger` / `sleeper` / `subject` / `member`）与六台探针**只在验收镜像里**：它们量的是服务，
/// 不是"机器起不起得来"。
///
/// **`spot` 与 `scenes` 是两件事**（用户原话："以后真正的程序放哪里，现在真的很不清晰"）：
/// `spot` 说"这一台是什么"（产品 / 探针 / 压测台），`scenes` 说"这次装不装它"。两者今天**不重合**
/// ——六位常客的 `spot` 是 [`Spot::Guest`]（它们是产品侧的客人，**不是探针**），却不进 `product` 景。
/// "真正要发出去的是哪几条"只有一个出处：下面每一行的 `scenes`。
///
/// **本表一行一台，`rustfmt` 请绕开**：默认那套会把每台摊成十几行（`Plan` 再嵌一层），
/// 于是"哪几台进哪张镜像"就没法一眼扫完——而这张表**就是**给人扫的。列宽由人手对齐。
#[rustfmt::skip]
pub const ALL: &[Row] = &[
    Row { name: "root", kind: ProgramKind::Supervisor, spot: Spot::Domain, scenes: &["root", "product"], plan: None },
    // 调试回显：**U 态**（最小特权）——它只走 `env` 的调试面（`DebugCall`），
    // 够不着建域那道 S 态门。**位次 18**（原 17）：`probe-bound` 那一台要赶在它前面起
    // ——喂键那一套等的是探针收尾，而它一退场整台机器就开始收场（见 `soak` 的门）。
    Row { name: "echo", kind: ProgramKind::User, spot: Spot::Console, scenes: &["root", "product"], plan: Some(Plan { order: 18, announce: Announce::None, tokens: &[], channels: &[], needs: None, board: true, operator: true, bind: true, holds_tree: false, eyes: None, died: E_ECHO }) },
    // 客人：**U 态**（与 `echo` 同一档）——按名字找到一个服务、说一句话。铸孔、交出、
    // 一问一答都不需要 S 态，故最小特权的域也能用板。
    Row { name: "guest", kind: ProgramKind::User, spot: Spot::Guest, scenes: &["root"], plan: Some(Plan { order: 6, announce: Announce::None, tokens: &[], channels: &[], needs: None, board: true, operator: true, bind: true, holds_tree: false, eyes: None, died: E_GUEST }) },
    // 过客：**U 态**（同上）——起来、挂一个名字、**直接死**（不说再见）。它与 `guest` 只差
    // 少说那一句退场：板上那两本账的"死"判据读的都是"那一枚入口还答得出吗"（`Probe`）。
    Row { name: "passer", kind: ProgramKind::User, spot: Spot::Guest, scenes: &["root"], plan: Some(Plan { order: 7, announce: Announce::None, tokens: &[], channels: &[], needs: None, board: true, operator: false, bind: true, holds_tree: false, eyes: None, died: E_PASSER }) },
    // 房客：**U 态**（同上）——起来、占一条线、**直接死**。它与 `passer` 在线轴上同形：两位
    // 喂的都是"看出来的"那一档（板那本账 / 线那本账）。它领一枚门闩（`virtio_mmio@10001000`，
    // 1 号线——**一条没人要的线**）却从不映视图：领它只为"主人"这个说法是真的；占住线之后
    // 一句话不说就走，路由者靠 `sweep` 收掉它（读数 `router: line 1 = virtio_mmio@10001000`
    // 与 `router: vacate line=1`）。**照实记**：它从前占的是 11 号线（那时钟），第二台设备
    // 驱动上来之后那条线有主了，故换成 1 号线。
    Row { name: "lodger", kind: ProgramKind::User, spot: Spot::Guest, scenes: &["root"], plan: Some(Plan { order: 8, announce: Announce::Channel, tokens: &[], channels: &["records"], needs: Some(LODGER_WANTS), board: false, operator: true, bind: true, holds_tree: false, eyes: None, died: E_LODGER }) },
    // 线路由者（中断面域）：**U 态**——实测（本行下面那条注里的疑点已经量掉）：它只读
    // PLIC 的寄存器（banner 里 PLIC 的 PMP 是 **S/U (R,W)**）、claim/complete、铸孔、挂组，
    // 全都不需要 S 态；它那枚铃是**内核给的**（铸铃那一格才是 S 态，本域不铸）。
    Row { name: "router", kind: ProgramKind::User, spot: Spot::Service, scenes: &["root", "product"], plan: Some(Plan { order: 3, announce: Announce::Channel, tokens: &[], channels: &["records"], needs: Some(ROUTER_WANTS), board: true, operator: true, bind: true, holds_tree: false, eyes: None, died: E_ROUTER }) },
    // 串口驱动：**U 态**（同上）——持有 `serial@10000000`（PMP 也是 S/U (R,W)），把"收到
    // 字节就拉线"打开。**照实记**：这两格从前写 `Supervisor` 是照搬旧树，理由（"要读写
    // 寄存器"）与 banner 里那张 PMP 对不上；改成 U 态之后两道门（examine / soak）照旧全过。
    Row { name: "uart", kind: ProgramKind::User, spot: Spot::Service, scenes: &["root", "product"], plan: Some(Plan { order: 4, announce: Announce::Channel, tokens: &[], channels: &["records"], needs: Some(UART_WANTS), board: true, operator: true, bind: true, holds_tree: false, eyes: None, died: E_UART }) },
    // 第二台设备驱动：**U 态**（同上）——持有 `rtc@101000`（11 号线），武装闹钟、到点自己
    // 拉线；客人定的闹钟到点就清掉那一格、把"那一声"推回去。**它是"抽象等第二个实例"的那个
    // 第二例**：线那四格、配给、设备面这一整套在第二台真设备上再走一遍，**服务面**也在它上面
    // 第二次落地（`uart` 那一面只有一个方向，它这一面两个方向都有）。
    Row { name: "rtc", kind: ProgramKind::User, spot: Spot::Service, scenes: &["root", "product"], plan: Some(Plan { order: 5, announce: Announce::Channel, tokens: &[], channels: &["records"], needs: Some(RTC_WANTS), board: true, operator: true, bind: true, holds_tree: false, eyes: None, died: E_RTC }) },
    // 客人：**U 态**（同上）——`/device/rtc` 那面服务的第一位用家：问一声现在几点、约一个时刻
    // （失败域那两格也各走一趟，见 `harness/src/sleeper.rs`），等到那一声就退场。
    Row { name: "sleeper", kind: ProgramKind::User, spot: Spot::Guest, scenes: &["root"], plan: Some(Plan { order: 9, announce: Announce::None, tokens: &[], channels: &[], needs: None, board: true, operator: true, bind: true, holds_tree: false, eyes: None, died: E_SLEEPER }) },
    // 主体（**U 态**）：身份服务的第一位真客人——问自己是谁、查父（三态）、验自反与否、
    // 派生一条自己的子身份、再越权趟一次（读数见 `harness/src/subject.rs` 头注）。
    Row { name: "subject", kind: ProgramKind::User, spot: Spot::Guest, scenes: &["root"], plan: Some(Plan { order: 10, announce: Announce::None, tokens: &[], channels: &[], needs: None, board: false, operator: true, bind: true, holds_tree: false, eyes: None, died: E_SUBJECT }) },
    // 盟友（**U 态**）：结盟服务的第一位真客人——立两枚盟、进进出出、验幂等与第三态，
    // 再用派生的第二条身份验"同一枚盟里有两位"（读数见 `harness/src/member.rs` 头注）。
    Row { name: "member", kind: ProgramKind::User, spot: Spot::Guest, scenes: &["root"], plan: Some(Plan { order: 11, announce: Announce::None, tokens: &[], channels: &[], needs: None, board: false, operator: true, bind: true, holds_tree: false, eyes: None, died: E_MEMBER }) },
    // 编排域：**S 态**——它要 mint/hatch（那是"建域 + 产线程 + 放行"整套），且整台机器
    // 的服务都由它起。它自己由**引导域**起：内核把 initrd 区与配对块只读借映进引导域，
    // 之后"这批字节交给谁"由域自己决定（见 `platform/devices.rs::supply_initrd`）。
    Row { name: "system", kind: ProgramKind::Supervisor, spot: Spot::Domain, scenes: &["root", "product"], plan: None },
    // 负证客人（**U 态**）：一位**没有身份**的任务去撞树的门（`Program::bind = false`）——
    // 门禁那条"没绑身份 ⇒ 拒绝"的判据在真机上的反例。读数见 `harness/src/probe_denied.rs`。
    Row { name: "probe-denied", kind: ProgramKind::User, spot: Spot::Probe, scenes: &["root"], plan: Some(Plan { order: 12, announce: Announce::None, tokens: &[], channels: &[], needs: None, board: false, operator: true, bind: false, holds_tree: false, eyes: None, died: E_PROBE }) },
    // 第二种负证（**U 态**）：**有身份**、但那一格归别人（`Rule::Owner`）⇒ 也拒。
    Row { name: "probe-owner", kind: ProgramKind::User, spot: Spot::Probe, scenes: &["root"], plan: Some(Plan { order: 14, announce: Announce::None, tokens: &[], channels: &[], needs: None, board: false, operator: true, bind: true, holds_tree: false, eyes: None, died: E_PROBE_OWNER }) },
    // 规矩那一格的证客（**U 态**）：有身份的一台把 `Is` / `Under` / `In` 三条规矩落下去，
    // 先以自己试（正证），再换一位代表试（负证 + "看支不看相等"）。读数见
    // `harness/src/probe_rule.rs`。
    Row { name: "probe-rule", kind: ProgramKind::User, spot: Spot::Probe, scenes: &["root"], plan: Some(Plan { order: 15, announce: Announce::None, tokens: &[], channels: &[], needs: None, board: false, operator: true, bind: true, holds_tree: false, eyes: None, died: E_PROBE_RULE }) },
    // 另一位客人（**U 态**）：**有身份**地去用别人立了规矩的那两格 ⇒ 都该拒。
    // 那是"第二道门"的反例（第一道由 `probe-denied` 量）。读数见 `probe_rule_other.rs`。
    Row { name: "probe-rule-other", kind: ProgramKind::User, spot: Spot::Probe, scenes: &["root"], plan: Some(Plan { order: 16, announce: Announce::None, tokens: &[], channels: &[], needs: None, board: false, operator: true, bind: true, holds_tree: false, eyes: None, died: E_PROBE_OTHER }) },
    // 会死的持有者（**U 态**）：落一块**声明归自己**的门牌然后直接死——好让下一台接手。
    Row { name: "probe-lease", kind: ProgramKind::User, spot: Spot::Probe, scenes: &["root"], plan: Some(Plan { order: 13, announce: Announce::None, tokens: &[], channels: &[], needs: None, board: false, operator: true, bind: true, holds_tree: false, eyes: None, died: E_PROBE_LEASE }) },
    // **上界的证客**（**U 态**）：一位故意的坏客人——推一页 + 1（该被拒），再推一枚不合族的帧
    // 到树的门上（门该把它吞下去、照旧答得出）。读数见 `harness/src/probe_bound.rs`。
    Row { name: "probe-bound", kind: ProgramKind::User, spot: Spot::Probe, scenes: &["root"], plan: Some(Plan { order: 17, announce: Announce::None, tokens: &[], channels: &[], needs: None, board: false, operator: true, bind: true, holds_tree: false, eyes: None, died: E_PROBE_BOUND }) },
    // 压测台的两个（`harness/src/`）：`churn` = 受害者——U 态，不停地在
    // "挂着"与"在台上"之间换（那正是"他杀偶发不生效"那道缝要的状态）；`rig` = 台主——
    // S 态，**景 `rig` 的引导镜像**，反复造/杀它。
    Row { name: "churn", kind: ProgramKind::User, spot: Spot::Rig, scenes: &["again"], plan: None },
    Row { name: "rig", kind: ProgramKind::Supervisor, spot: Spot::Rig, scenes: &["rig"], plan: None },
    // 忙机台的另外两个：`busy` = 占核者——U 态，纯自旋**永不落核**；`load` = 台主——
    // S 态，**景 `load` 的引导镜像**。它把每一颗核钉住，好让「到点兑现」这条债
    // 在树内第一次变得可测（`soak`/`rig` 里总有核空闲，空闲核会替全局兑现到点）。
    Row { name: "busy", kind: ProgramKind::User, spot: Spot::Rig, scenes: &["load"], plan: None },
    Row { name: "park", kind: ProgramKind::User, spot: Spot::Rig, scenes: &["load"], plan: None },
    // 他杀台的握手版受害者（rig A）：无限挂在自己的孔上、由台主 push 唤醒——上台/离核的
    // 转折点因此由台主定（旧版 `churn` 是"放行即跑"，量到的全是快路径）。
    Row { name: "hang", kind: ProgramKind::User, spot: Spot::Rig, scenes: &["rig"], plan: None },
    Row { name: "load", kind: ProgramKind::Supervisor, spot: Spot::Rig, scenes: &["load"], plan: None },
    // 到点台的打点者：**S 态**（与两个台主同档），**景 `beat` 的引导镜像**。
    // 它不造任何东西，只量"睡到绝对点"漂不漂（两段对照，见程序头注）。
    Row { name: "beat", kind: ProgramKind::Supervisor, spot: Spot::Rig, scenes: &["beat"], plan: None },
    // 重启台：**S 态**（要 mint/hatch 那道门），**景 `again` 的引导镜像**。
    // 它在同一张表、同一行上把"起 → 停 → 放下 → 再起"走三遍（协议 §六 的"重发"）。
    Row { name: "again", kind: ProgramKind::Supervisor, spot: Spot::Rig, scenes: &["again"], plan: None },
    // 共享组台的两个（`harness/src/`）：`waiter` = 等待者——U 态，把台主
    // 给的那枚孔挂进**共享组**并等组键（**多个等待者挂同一只键**）；`group` = 台主——
    // S 态，**景 `group` 的引导镜像**：一次投信，看两个等待者是不是**都醒**，
    // 以及那条消息是不是**只归一个人**。
    Row { name: "waiter", kind: ProgramKind::User, spot: Spot::Rig, scenes: &["group"], plan: None },
    Row { name: "group", kind: ProgramKind::Supervisor, spot: Spot::Rig, scenes: &["group"], plan: None },
];
