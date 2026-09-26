//! program — **一台程序是什么**：它的全部装配声明，都写在它自己那份 `program.rs` 里。
//!
//! ```text
//!   每台自己的声明（pub static PROGRAM）──┐
//!                                        ├─▶ PROGRAMS（只有引用，没有第二份定义）
//!   image 按 scenes/entry 挑镜像 ─────────┘
//!   编排域按 order 起、按 Program::assemble 装配
//! ```
//!
//! # 判据（这一层现在只有一件事）
//!
//! **`Program` 是程序装配声明的唯一来源。** 这里没有 `Row`、没有 `Plan`、没有"装配单 + 需求单
//! + 关系单"三张表：一台程序的 `name` / `kind` / `spot` / `scenes` / `entry` / `order` /
//! `board` / `operator` / `bind` / `holds_tree` / `eyes` / `died` / `setup` 全在它自己那一份
//! `program.rs` 里，本文件只把它们的**引用**摆成一张表（[`PROGRAMS`]）。
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
//! **那是报警器，不是隐患**——`Program::assemble` 故意不住这里（它住 `system/mod.rs`）。

use env::ProgramKind;
use env::supply::Need;
use env::wire::Eyes;

// ── 声明本身 ─────────────────────────────────────────────────────────

/// 装配失败的编号——`env::Reason` 的别名。
///
/// 装配编号只是"域自己的小整数"那一族（见 [`env::exit`] 的头注），不另立类型；名字留着是因为
/// 这张表通篇讲的是"哪一台、死在第几步"。
pub type Died = env::Reason;

/// **这一台是什么**——角色。与"进哪张镜像"（[`Program::scenes`]）分开的一格：两者**不重合**
/// （常客也在产品侧，却不进产品镜像），故不许拿它当"装不装"用。
///
/// **照实记（变体名一律不用景名）**：从前有一格叫 `Product`、注释写着"去掉它，机器不成机器"
/// ——加 `product` 那一景当场把这句话证伪了（六位常客全去掉，机器照起照停）。故按**真实角色**
/// 重分：谁进哪张镜像只有 [`Program::scenes`] 一处说。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Spot {
    /// 两个**域**：引导域（`root`）与编排域（`system`）——机器本身的骨架。
    Domain,
    /// **常驻服务**：三台驱动 ＋ 三枚服务（持树者 / 名册 / 盟册）。
    Service,
    /// **调试回显**（`echo`）：只走 `env` 调试面的那一条；产品镜像里排**最后**，编排域等它退场。
    Console,
    /// **常客**：产品侧的客人——量服务用的；去掉它，机器照转。
    Guest,
    /// **只读数**的探针：负证那一族，量的是门禁答得对不对。
    Probe,
    /// 压测台与它们的受害者：**整台替换引导镜像**，与验收场景没有交集。
    Rig,
}

/// **一台程序**：它的身份、它进哪张镜像、它在装配图里的边、它起手要什么。
///
/// **它没有"从哪儿来"那一格**：每一条的身子的来路只有一种——按 `name` 去清单里挑镜像、
/// 建域、产线程（`Control::spawn`）。
#[derive(Clone, Copy)]
pub struct Program {
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
    /// 编排域的起手位次；`None` = **不由编排域起**（引导域 / 编排域自己 / 压测台）。
    pub order: Option<u8>,
    /// 上不上板（客人交回的那一枚由编排域转授过去）。
    pub board: bool,
    /// 接不接**持树者那棵树**。
    pub operator: bool,
    /// 放行前给不给**身份**（`false` = 没绑身份，撞门该被拒——负证客人就是靠它）。
    pub bind: bool,
    /// 它**是不是持树者**（起来时把提示之路交给生我者）。
    pub holds_tree: bool,
    /// **它是哪一双眼睛**（`None` = 不是：绝大多数行都不是）。
    pub eyes: Option<Eyes>,
    /// 死在装配哪一步的号。`order` 为 `None` 的那几台不读这一格（写 [`env::EXIT_OK`]）。
    pub died: Died,
    /// 实例化它要多做的那几手（资源 / 通信）。
    pub setup: &'static [Setup],
}

/// 实例化一台要多做的一手。
///
/// **它不负责 start**：`Channel` 在手（`Control::spawn` 之后）装泊位；`Need` 要等服务起来
/// 之后才递（`Control::wire`）。两件都由 `Program::assemble` 按次序落到 `Control` 那两手上。
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
#[path = "user/echo/program.rs"]
pub mod echo;
#[path = "driver/router/program.rs"]
pub mod router;
#[path = "driver/uart/program.rs"]
pub mod uart;
#[path = "driver/rtc/program.rs"]
pub mod rtc;
/// harness 那 22 台（**测具**）：它们的身子住隔壁那个 crate，而其中 12 台**由编排域起**
/// ——编排域要按 `order` / `board` / `bind` / `died` 起它们，故声明必须由本 crate 编译。
/// `harness` 依赖 `programs`，反向不可能。故这一族的声明住这里（一份，不拆 22 份：
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
    &echo::PROGRAM,
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
