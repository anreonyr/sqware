//! program — **一台程序是什么**：它的全部装配声明，都写在它自己那份 `program.rs` 里。
//!
//! ```text
//!   每台自己的声明（pub static PROGRAM）──┐
//!                                        ├─▶ PROGRAMS（只有引用，没有第二份定义）
//!   image 按 scenes/entry 挑镜像 ─────────┘
//!   编排域按 order 起、按 Assembly::assemble 装配
//! ```
//!
//! # 判据（这一层现在只有一件事）
//!
//! **`Program` 是程序装配声明的唯一来源。** 这里没有 `Row`、没有 `Plan`、没有"装配单 + 需求单
//! + 关系单"三张表：一台程序的三块——**身份**（[`Identity`]）、**装配关系**（[`Relation`]）、
//! **需求**（[`Demand`]）——全在它自己那一份 `program.rs` 里，本文件只把它们的**引用**摆成一张表
//! （[`PROGRAMS`]）。
//!
//! # 三块为什么分开（拆毒那一刀）
//!
//! **照实记**：从前这 13 格平铺在一个结构体上，谁都能随手读哪一格——"它是谁"、"它跟谁有边"、
//! "它起手要什么"三件事混在同一层，于是**没有一处改动看得出会牵动谁**。拆成三块之后，每块各有
//! 各的读者：宿主那侧（`crates/image` 打包）**只读身份**（且只走 [`Program`] 上那四个只读面）；
//! 编排域装配那一趟读装配关系与需求，并且**按块走各自那几手**（见 `system::Assembly` 与
//! `system/mod.rs` 的头注）。一块里的格不再跨块乱叫。
//!
//! # 本文件为什么是"宿主安全"的
//!
//! `crates/image`（**宿主** std 程序）要读同一张表决定"哪几台进哪张镜像"，而 `programs` /
//! `harness` 拖着 `protocol → runtime`（riscv 内联汇编，宿主上编不过）⇒ 它**不能**依赖
//! `programs`。故本模块——连同它 `#[path]` 拉进来的每一份声明——**只许引 `env`**，由
//! `crates/image` 用一行 `#[path]` 文本包含（见那边的 `mod program;`）。
//!
//! **纪律**：本模块与各 `program.rs` / `decl/harness.rs` 里出现任何 `protocol::` / `runtime::` /
//! `crate::driver::` 之类的引用，`cargo image` 就会连带把 riscv 代码拉进宿主构建而当场红。
//! **那是报警器，不是隐患**——`Assembly::assemble` 故意不住这里（它住 `system/mod.rs`）。

use env::ProgramKind;
use env::supply::Need;
use env::wire::Eyes;

// ── 声明本身 ─────────────────────────────────────────────────────────

/// 装配失败的编号——`env::Reason` 的别名。
///
/// 装配编号只是"域自己的小整数"那一族（见 [`env::exit`] 的头注），不另立类型；名字留着是因为
/// 这张表通篇讲的是"哪一台、死在第几步"。
pub type Died = env::Reason;

/// **这一台是什么**——角色。与"进哪张镜像"（[`Identity::scenes`]）分开的一格：两者**不重合**
/// （常客也在产品侧，却不进产品镜像），故不许拿它当"装不装"用。
///
/// **照实记（变体名一律不用景名）**：从前有一格叫 `Product`、注释写着"去掉它，机器不成机器"
/// ——加 `product` 那一景当场把这句话证伪了（六位常客全去掉，机器照起照停）。故按**真实角色**
/// 重分：谁进哪张镜像只有 [`Identity::scenes`] 一处说。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Spot {
    /// 两个**域**：引导域（`root`）与编排域（`system`）——机器本身的骨架。
    Domain,
    /// **常驻服务**：三台驱动 ＋ 三枚服务（持树者 / 名册 / 盟册）。
    Service,
    /// **控制台那一台**（`canonical`）：本域扮**终端那一侧的行规程**（ECHO / ERASE / KILL / EOF）；
    /// 产品镜像里排**最后**，编排域等它退场才收场。
    Console,
    /// **常客**：产品侧的客人——量服务用的；去掉它，机器照转。
    Guest,
    /// **只读数**的探针：负证那一族，量的是门禁答得对不对。
    Probe,
    /// 压测台与它们的受害者：**整台替换引导镜像**，与验收场景没有交集。
    Rig,
}

/// **一台程序**：它的身份、它在装配图里的边、它起手要什么——**三块分开**。
///
/// ```text
///   Identity   它是谁（清单名 / 特权级 / 角色 / 进哪几张景 / 是不是引导镜像）
///   Relation   它跟谁有边（位次 / 存在信号 / 树 / 身份 / 持树者 / 眼睛）
///   Demand     它起手要什么（来源 / 死在第几步 / 那几手 setup）
/// ```
///
/// **它没有"代码在哪儿"那一格**：只有"从哪本账取这一段字节"（[`Demand::origin`]）——内核按 ELF
/// 段现读，故字节从哪来这件事窄到一句话（见 [`Origin`]）。
#[derive(Clone, Copy)]
pub struct Program {
    /// **身份**：它是谁（宿主那侧只读这一块）。
    pub identity: Identity,
    /// **装配关系**：编排域把它接进来时那几条边。
    pub relation: Relation,
    /// **需求**：它起手要什么、身子从哪来、死在装配哪一步。
    pub demand: Demand,
}

impl Program {
    // ── 身份那四样只读面：宿主那侧（`crates/image`）与装配者都不伸手进块里 ──
    //
    // **照实记（为什么开这四扇门）**：`crates/image` 原先直接读 `p.name` / `p.kind` /
    // `p.scenes` / `p.entry`——拆块那一刀一到，那四处就会**跟着块的形状碎**，而它不是本仓的
    // 装配方（宿主只打包），不该被卷进"三块怎么分"这件事。故给 `Program` 留这四条窄面：
    // 宿主读的永远是"它是谁"，块再怎么挪，这四行不动。

    /// 清单名。
    pub fn name(&self) -> &'static str {
        self.identity.name
    }

    /// 装成哪种空间。
    pub fn kind(&self) -> ProgramKind {
        self.identity.kind
    }

    /// 进哪几张引导镜像（景名）。
    pub fn scenes(&self) -> &'static [&'static str] {
        self.identity.scenes
    }

    /// 它是哪几张景的引导镜像。
    pub fn entry(&self) -> &'static [&'static str] {
        self.identity.entry
    }
}

/// **身份**：这一台是谁——清单名、装成哪种空间、角色、进哪几张景、是不是引导镜像。
///
/// **它是谁与它怎么被接进来是两件事**：`crates/image` 那台宿主只读这一块（且只走
/// [`Program::name`] 那四条窄面），装配关系与需求一概与打包无关。
#[derive(Clone, Copy)]
pub struct Identity {
    /// 清单名（也是 cargo 的 bin 名去掉 `prog-`，见 `crates/image` 那条照实记）。
    pub name: &'static str,
    /// 装成哪种空间。
    pub kind: ProgramKind,
    /// 这一台是什么：**角色**（见 [`Spot`]）。
    pub spot: Spot,
    /// 进哪几张引导镜像（**景名**）——**次序即装载次序**。
    pub scenes: &'static [&'static str],
    /// **它是哪几张景的引导镜像**（多数为空）。一个景存在 ⇔ 它有一条引导镜像，故这张表
    /// 也是"有哪些景"的唯一一览（从前那格 `ENTRY` 并进了这里）。
    pub entry: &'static [&'static str],
}

/// **装配关系**：编排域把它接进来时那几条边。
///
/// **这一块只有装配者读**：位次 / 存在信号 / 树 / 身份 / 持树者 / 眼睛——都是"这一台与那一台
/// 之间有一条什么边"，与它自己是谁（[`Identity`]）、起手要什么（[`Demand`]）分开。
#[derive(Clone, Copy)]
pub struct Relation {
    /// 编排域的起手位次；`None` = **不由编排域起**（引导域 / 编排域自己 / 压测台）。
    pub order: Option<u8>,
    /// **要不要存在信号**（原 `board`）。
    ///
    /// **照实记（改名那一刀）**：这一格原先叫"上不上板"，问的是 `Board` 那一台；而板那一侧已经
    /// 退成**一枚死信号传感器**（"它没了"这件事要有人报到编排域，见
    /// [`system::board`](crate::system::board)）——"上板"这个说法只剩历史。故按它真正答的那句话
    /// 改名：**要不要存在信号**。铸道与接板都只读这一格（`Watch::of` 铸道、`Bridge::attach` 接上）。
    pub presence: bool,
    /// 接不接**持树者那棵树**。
    pub operator: bool,
    /// 放行前给不给**身份**（`false` = 没绑身份，撞门该被拒——负证客人就是靠它）。
    pub bind: bool,
    /// 它**是不是持树者**（起来时把提示之路交给生我者）。
    pub holds_tree: bool,
    /// **它是哪一双眼睛**（`None` = 不是：绝大多数行都不是）。
    pub eyes: Option<Eyes>,
}

/// **需求**：实例化它要多做的那几手、它的身子从哪来、它死在装配哪一步的号。
///
/// **这一块只有装配者读**，且三格答的是同一句话的两面："把它弄起来要动用什么"。
#[derive(Clone, Copy)]
pub struct Demand {
    /// **它的身子从哪来**（来源那一格，见 [`Origin`]）。
    ///
    /// **照实记（它为什么与 `setup` 同块）**：来源与 `setup` 答的是同一件事——"把它弄起来要动用
    /// 什么"：前者是**那一段字节**（唯一消费者是装配时 `service::mint` 那一行），后者是**门闩与
    /// 通道**；两者都不属于"它是谁"，也不属于"它跟谁有边"。
    pub origin: Origin,
    /// 死在装配哪一步的号。`order` 为 `None` 的那几台不读这一格（写 [`env::EXIT_OK`]）。
    ///
    /// **照实记（它为什么与 `setup` 同块）**：`died` 是**这一台 `setup` 走不通时**的读数——
    /// 与起手那几手是同一件事的两面，故同块。
    pub died: Died,
    /// 实例化它要多做的那几手（资源 / 通信）。
    pub setup: &'static [Setup],
}

/// **程序来源**：这一台的身子的那一段字节**从哪本账里取**。
///
/// # 它为什么窄到只有一句话
///
/// **一段字节就是全部交接面**：`UnitCall::Build` 的正文写着"**镜像字节不被拷走**，内核按 ELF
/// 段现读 `elf` 那几页"（`crates/env/src/fid.rs:389`）⇒ 任何来源都只是"给内核一段 `&[u8]`"，
/// **无一字节需要跨域搬运**：引导域手里那份 initrd 清单、编排域领到的那段只读视图，是**同一批
/// 物理页的各自 VA**。故来源这一维只落一句话：取字节那一面（`system::source::Source::image`），
/// 不牵动装配的其余任何一格——帧里也从不带镜像。
///
/// **今天只有 [`Origin::Initrd`] 一档是真的**：[`Origin::Storage`] 那一台（盘 / 文件系统）
/// **不存在**——它只会答 `Error::NoSource`（照实记：那一台不存在，不是"待实现的功能"）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Origin {
    /// **initrd 档**：boot 给的那块字节（清单与全部镜像都在里面，零拷贝借映）。
    Initrd,
    /// **存储档**：从盘 / 文件系统取——**今天不存在**（见本类型头注）。
    Storage,
}

/// 实例化一台要多做的一手。
///
/// **它不负责 start**：`Channel` 在手（`service::mint` 之后）装泊位；`Need` 要等服务起来
/// 之后才递（`service::wire`）。两件都由编排域装配那一趟按次序落到 `Control` 那两手上。
///
/// **"怎么算它起来了"由这一格推**（有 `Channel` ⇒ `Announce::Channel`）——故不给它另立一格。
#[derive(Clone, Copy)]
pub enum Setup {
    /// **资源**：这一台要一枚门闩（今天 = 收方那张需求单里的一条）。
    Need(Need),
    /// **通信**：这一台要开一条通道（今天只有 `records`）。放行前 `connect`，放行后按同一个
    /// 记号 `claim`——它交回那一枚就是"它起来了"的证据。
    Channel(&'static str),
}

// ── 每台自己的声明 ───────────────────────────────────────────────────
//
// **它们不在这份文件的自然模块树里**：那些目录（`driver/uart/`、`system/operator/`…）都拖着
// runtime / protocol 代码，`crates/image` 进不去。故只由 [`PROGRAMS`] 这一处按 `#[path]` 拉
// 进来一次——**唯一的声明点**。

#[path = "root/program.rs"]
pub mod root;
#[path = "system/program.rs"]
pub mod system;
#[path = "system/operator/program.rs"]
pub mod operator;
#[path = "system/principal/program.rs"]
pub mod principal;
#[path = "system/coalition/program.rs"]
pub mod coalition;
#[path = "user/canonical/program.rs"]
pub mod canonical;
#[path = "driver/router/program.rs"]
pub mod router;
#[path = "driver/uart/program.rs"]
pub mod uart;
#[path = "driver/rtc/program.rs"]
pub mod rtc;
/// harness 那 23 台（**测具**）：它们的身子住隔壁那个 crate，而其中 13 台**由编排域起**
/// ——编排域要按 `order` / 存在信号 / `bind` / `died` 起它们，故声明必须由本 crate 编译。
/// `harness` 依赖 `programs`，反向不可能。故这一族的声明住这里（一份，不拆 23 份：
/// "紧挨着身子"对身子不在本 crate 的那几台本来就不成立，不假装）。
#[path = "decl/harness.rs"]
pub mod harness;

/// **装配表**：镜像里可能有的全部程序。**次序是硬事实**——它就是装载次序（`ROOT_OFFSET`
/// 按位次算），且各景按 [`Program::scenes`] 过滤 ⇒ 加一台要想清楚放哪。
///
/// **本表只有引用**：每一台的声明都在它自己那份 `program.rs` 里，这里不再写第二遍。
///
/// **本表一行一台，`rustfmt` 请绕开**：默认那套会把每台摊成十几行，于是"哪几台进哪张镜像"
/// 就没法一眼扫完——而这张表**就是**给人扫的。
#[rustfmt::skip]
pub const PROGRAMS: &[&Program] = &[
    &root::PROGRAM,
    // 三枚服务（持树者 / 名册 / 盟册）：各自一个 bin、一个域，与其他每一台同一条 `mint` 路。
    &operator::PROGRAM,
    &principal::PROGRAM,
    &coalition::PROGRAM,
    &canonical::PROGRAM,
    // 客人 / 过客 / 房客：量服务用的（去掉机器照转）。
    &harness::GUEST,
    &harness::PASSER,
    &harness::LODGER,
    // 三台驱动。
    &router::PROGRAM,
    &uart::PROGRAM,
    &rtc::PROGRAM,
    &harness::SLEEPER,
    &harness::SUBJECT,
    &harness::MEMBER,
    &system::PROGRAM,
    &harness::PROBE_DENIED,
    &harness::PROBE_OWNER,
    &harness::PROBE_RULE,
    &harness::PROBE_RULE_OTHER,
    &harness::PROBE_LEASE,
    &harness::PROBE_BOUND,
    // 控制面那位真客人（`/sys/control`）：**排在 `canonical` 之前**，见它自己那份声明。
    &harness::PROBE_CONTROL,
    // 操作面那一族（`/sys/operator/{part,land,…}`）：**两位一对**——`gate` 拿控制面会话把七格
    // 验一遍并取回那一枚入口、铺好试验场；`land` 只持 `land` 一位（时序见各自那份声明）。
    &harness::PROBE_OPERATOR_GATE,
    &harness::PROBE_OPERATOR_LAND,
    // 压测台与它们的受害者（整台替换引导镜像）。
    &harness::CHURN,
    &harness::RIG,
    &harness::BUSY,
    &harness::PARK,
    &harness::HANG,
    &harness::LOAD,
    &harness::BEAT,
    &harness::AGAIN,
    &harness::WAITER,
    &harness::GROUP,
];

/// 清单条数上界与注册表条数必须相容（见 [`env::manifest::MAX_PROGRAMS`] 的头注）。
///
/// **这一条就是从前那个"数出来的数"的替身**：加一台超过上界 ⇒ 当场编不过，不可能静默卡住。
const _: () = assert!(PROGRAMS.len() <= env::manifest::MAX_PROGRAMS);
