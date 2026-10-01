//! program — **一台程序是什么**：它的全部装配声明，都写在它自己那份 `program.rs` 里。
//!
//! **照实记（层六第一步：`program.rs` → `program/mod.rs`）**：这一份从 `src/program.rs` 收进
//! **同名目录**（`src/program/mod.rs`）——**模块名一个字没改**，故全仓引用一处都没动；动的是
//! **两处 `#[path]` 的基准**：`crates/image` 那一行（`../../../programs/src/program/mod.rs`）
//! 与本文件里那 **11 条**拉声明的 `#[path]`（相对本文件所在目录 ⇒ 各加一层 `../`）。
//! **为什么先收目录再拆**：下一刀要把这一份按职责拆成 `order.rs` / `catalog.rs`（计划里那一刀），
//! 而拆之前得先有"目录"这个位置——**先搬位置，再拆内容**，两步各验一次。
//!
//! ```text
//!   每台自己的声明（pub static PROGRAM）──┐
//!                                        ├─▶ PROGRAMS（只有引用，没有第二份定义）
//!   image 按 wanted_by/entry 挑镜像 ─────────┘
//!   编排域按 order 起、按 Assembly::assemble 装配
//! ```
//!
//! # 层四施工图（`Program` → `UnitFile`：**逐格说清它去 systemd 的哪一个键**）
//!
//! **照实记（这一张表是量出来的，不是想出来的）**：13 格逐格点过写者与读者（数写在表里），
//! 落法三种：**改名**（同一个东西、systemd 有现成的键）／**合**（两格答的是同一件事）／
//! **待裁**（systemd 里没有对应，而它**不是死格**）。
//!
//! | 今天这一格 | 写者 | systemd 那一侧 | 怎么落 |
//! |---|---|---|---|
//! | `Identity::name` | 35 | unit 名 | 不动 |
//! | `Identity::kind` | 35 | **文件后缀**（`.service` / `.target`） | 不动（`SCENE` 那一台已经是 `Target`） |
//! | `Identity::space` | 35 | （无对应：域这一层） | 不动 |
//! | `Identity::wanted_by` | 22 | **`WantedBy=`**（景名即 target 名） | **已落**（原名 `scenes`） |
//! | `Identity::entry` | **7** | （领头那一台：systemd 没有对应） | **不合**（量出来是两件事，见下） |
//! | `Relation::after` | 23 | **`After=`** | **已落**（原名 `deps`） |
//! | `Relation::restart` | 23 | **`Restart=`**（寿命那一档） | **已落**（原名 `ending`） |
//! | `Relation::bind` | 23（22 `true` / 1 `false`） | （无对应） | **待裁** |
//! | `Demand::died` | 23（＋ 28 枚 `E_*` 常量） | （无对应） | **待裁** |
//! | `Demand::setup` | 9 | **`Type=notify`** | 收成 **`supply`**（8 处 `Ready` 推得出来） |
//!
//! ## 两处待裁的账（量过了，等一句裁定）
//!
//! · **`died`**：23 处写它，读者两处——装配者 `fail()` 报"这一台死在装配哪一步"用它，而**每一台
//!   自己的起手失败码用的就是同一个常量**（`driver/fail.rs` 头注："号与域名在装配表上是同一格的
//!   两半"）。**全仓没有一处按号断言**（探针只报 `EXIT_OK`）⇒ 撤它丢的是"用号认出台"这条读数
//!   （而"哪一台"那一行**已经不设门、一定在**）。**它不是死格**，故不擅自撤。
//! · **`bind`**：22 处 `true`、**1** 处 `false`（负证那一台**明写**——默认值翻过来之后"唯一那个
//!   反例不写"那个形状就没了）。读者一处（装配者放行前绑身份）。**撤它 = 改"身份由谁发"那条路**，
//!   不是改声明 ⇒ 等裁定。
//!
//! ## 落法（一刀一格，每刀同一套验收）
//!
//! ① `deps → after`（纯改名）；② `restart → restart`（纯改名）；③ `wanted_by ＋ entry → wanted_by`（合，
//! 动 `crates/image` 那两处读者）；④ `setup → supply`（`Ready` 推得出来、`Machine` 只有设备账那一台）；
//! ⑤ 两处待裁按裁定落。**每刀都走**：`cargo check --workspace` ＋ release/debug 起 ＋ `scene root`
//! qtest ＋ 读数 A/B（16 条 ＋ `system: gone` 23 行 ＋ 装配次序逐字）。
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

// ── 声明本身 ─────────────────────────────────────────────────────────

/// 装配失败的编号——`env::Reason` 的别名。
///
/// 装配编号只是"域自己的小整数"那一族（见 [`env::exit`] 的头注），不另立类型；名字留着是因为
/// 这张表通篇讲的是"哪一台、死在第几步"。
pub type Died = env::Reason;

// **照实记（`Spot` 那一格退场：29 处写、0 处读）**：它叫"角色"，答"这一台是什么"
// （域 / 常驻服务 / 控制台 / 常客 / 探针 / 压测台）——而**全仓没有一个读者**：打包那一侧按
// `wanted_by` / `entry` 挑镜像，装配那一侧按 `Relation` 那几条边走，`Spot` 一次都没被问过。
//
// **它从前不是死格，而是一格"按角色推事实"的钩子**——那正是最贵的那种格：`Start::code`
// 那三组硬编码就是按角色推出来的（量与代价见 `crates/env/src/fail.rs` 那一族的照实记），
// 而它自己的注释里也早写着"**不许拿它当'装不装'用**"。**原话留档**：这一格最早有一个变体叫
// `Product`，注释写着"去掉它，机器不成机器"——`product` 那一景一到，当场把这句话证伪（六位
// 常客全去掉，机器照起照停）。⇒ 按那条纪律办：**事实放在产生它的那一点**——"进哪几张镜像"住
// `wanted_by`、"先起谁"住 `after`、"它走了谁等"住 `restart`；角色这一句，谁都不读，故不写。

/// **一台程序**：它的身份、它在装配图里的边、它起手要什么——**三块分开**。
///
/// ```text
///   Identity   它是谁（清单名 / 单元类型 / 特权空间 / 进哪几张景 / 是不是引导镜像）
///   Relation   它跟谁有边（依赖 / 存在信号 / 身份）
///   Demand     它起手要什么（死在第几步 / 那几手 setup）
/// ```
///
/// **它没有"代码在哪儿"那一格，也没有"从哪本账来"那一格**：那一段字节由**这一景那本账**给
/// （[`crate::system::source`] 取字节那一面——内核按 ELF 段现读，镜像一个字节都不被拷走）。
/// 照实记（`Origin` 那一格为什么退场）见 [`Demand`] 底下那一段。
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
    // `p.wanted_by` / `p.entry`——拆块那一刀一到，那四处就会**跟着块的形状碎**，而它不是本仓的
    // 装配方（宿主只打包），不该被卷进"三块怎么分"这件事。故给 `Program` 留这四条窄面：
    // 宿主读的永远是"它是谁"，块再怎么挪，这四行不动。

    /// 清单名。
    pub fn name(&self) -> &'static str {
        self.identity.name
    }

    /// **它是哪一种单元**（[`Kind`]）。
    pub fn kind(&self) -> Kind {
        self.identity.kind
    }

    /// 装成哪种**空间**（S / U）。**它不是"单元类型"**——单元类型见 [`Kind`]。
    pub fn space(&self) -> ProgramKind {
        self.identity.space
    }

    /// 进哪几张引导镜像（景名）。
    pub fn wanted_by(&self) -> &'static [&'static str] {
        self.identity.wanted_by
    }

    /// 它是哪几张景的引导镜像。
    pub fn entry(&self) -> &'static [&'static str] {
        self.identity.entry
    }

    /// **它在不在这一趟装配单上**（`relation.after: Some`）。
    ///
    /// **宿主那一侧的第二组窄面**（照实记：本表原先只开四扇门，都取 [`Identity`]）：打包那一趟
    /// 仍只读前四样，而**校验那一趟**（[`order_scene`] 的调用点）要按这一格滤出"由编排域起的
    /// 那些台"——图的内部（`after` 的内容、[`SCENE`] 那条边）一律由本文件那两具读，宿主不碰。
    pub fn listed(&self) -> bool {
        self.relation.after.is_some()
    }
}

/// **单元类型**——systemd 那份模型里的 `.service` / `.target`（后缀定了型）。
///
/// **两个变体各有生产者与读者**（它不是枚举摆设）：
///
/// | 变体 | 是什么 | 生产者 | 读者 |
/// |---|---|---|---|
/// | [`Kind::Service`] | 要起的服务（有身子、进镜像） | 其余每一份声明（吃 [`Identity::DEFAULT`]） | 宿主打包（`crates/image` 按它滤）／装配那一趟 |
/// | [`Kind::Target`] | **只把几条边聚在一起的目标**（没有身子、不进任何镜像） | [`SCENE_UNIT`] 一处 | [`is_target`]（`order_scene` 与装配那一趟据它认那条"等这一趟走完"的边） |
///
/// **照实记（`Scope` 那一档为什么还没有）**：systemd 那一侧还有 `.scope`（外部造出来的、不被
/// 管理器起的单元）——本仓今天没有它的生产者（那 13 台"不在这一趟装配单上"的台，用
/// `after: None` 就说清了，而它们**仍然是一种服务**：有身子、进镜像）。**没有生产者的变体不立**
/// （`Spot` 那 29 写 0 读是上一课）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// 要起的服务：有身子（进镜像），由编排域按 `after` 起。
    Service,
    /// **目标**：没有身子、不进任何镜像，只把几条边聚在一起（"这一趟装配走完"就是它的内容）。
    Target,
}

/// **身份**：这一台是谁——清单名、**单元类型**（[`Kind`]）、装成哪种**空间**、
/// 进哪几张景、是不是引导镜像。
///
/// **它是谁与它怎么被接进来是两件事**：`crates/image` 那台宿主只读这一块（且只走
/// [`Program::name`] 那**四**条窄面：`name` / `space` / `wanted_by` / `entry`），装配关系与需求
/// 一概与打包无关。
///
/// **照实记（这块里为什么有"单元类型"）**：`kind` 那一格（[`Kind`]）答的是"它是哪一种单元"
/// ——宿主按它决定"要不要装进镜像"（目标单元没有身子），故它与"它是谁"同块；`space` 那一格
/// 答的是"它跑在哪个特权空间"（S/U），与 `env::ProgramKind` 那两个变体一一对应。
#[derive(Clone, Copy)]
pub struct Identity {
    /// 清单名（也是 cargo 的 bin 名去掉 `prog-`，见 `crates/image` 那条照实记）。
    pub name: &'static str,
    /// **单元类型**（[`Kind`]）：`Service` 是要起的服务，`Target` 只把几条边聚在一起——
    /// **没有身子、不进任何镜像**（今天只有一个：本文件末尾那个 [`SCENE_UNIT`]）。
    pub kind: Kind,
    /// 装成哪种**空间**（S / U）。
    ///
    /// **照实记（它从前叫 `kind`）**：那个名字与 [`Kind`]（单元类型：服务 / 目标）撞在一起——
    /// 一个说"它跑在哪个特权空间"，一个说"它是哪一种单元"，两件事。故按它真正答的那句话改名
    /// （与 `env::ProgramKind` 那两个变体 `Supervisor` / `User` 对应的是"空间"，不是"种类"）。
    pub space: ProgramKind,
    /// **`WantedBy=`**（systemd 同名那一格）：**哪几张景要我**（景名即 target 名——`SCENE` 那一台
    /// 就是 `.target`，见 [`Kind::Target`]）——**次序即装载次序**。
    ///
    /// **照实记（它原名 `scenes`，这一刀改成 `wanted_by`）**：`scenes` 说的是"我在哪几张景里"，
    /// 读起来像一句**关于自己的描述**；而这一格在装配那一侧说的是**别的东西**：**那几张景把我列进
    /// 它们的单子**（`crates/image` 按它挑镜像、装配者按它起台）。借 systemd 同名那一格说死：
    /// **谁要我**，不是"我在哪"。
    pub wanted_by: &'static [&'static str],
    /// **它是哪几张景的领头那一台**（**多数为空：全仓只有 7 处写它**）。一个景存在 ⇔ 它有一条
    /// 领头台，故这张表也是"有哪些景"的唯一一览（从前那格 `ENTRY` 并进了这里）。
    ///
    /// **照实记（它与 [`Identity::wanted_by`] 是两件事，故这一刀没有合它们）**：层四的施工图
    /// 里我原先写着"合 `scenes ＋ entry → wanted_by`"——**量了一遍就推翻了**：全仓写 `wanted_by`
    /// 的 **22** 处（哪几张景要我）、写 `entry` 的只有 **7** 处（`root` 那一台要 `root` 与
    /// `product` 两张、其余六处是压测台各自的景 `rig` / `load` / `beat` / `again` …）。一张是
    /// **"谁把我列进单子"**（22 台都答），一张是 **"我替哪张景拿主意"**（只有领头那几台答）
    /// ⇒ 合起来会把"绝大多数台不领头"这件事**藏进一个看起来人人都有写的字段里**。
    /// **故只改 `wanted_by` 那个名，`entry` 原样留着。**
    pub entry: &'static [&'static str],
}

/// **谁结束它**——这一台的寿命由谁定。
///
/// **照实记（为什么不按角色分类）**：按角色（当时的 `Spot`）反推"会不会自己走"，与 `Start::code`
/// 那三组硬编码是同一类错——那一刀**量出 8 处假读数**（名册死在树上报的是持树者…）。故按那条
/// 纪律办：**事实放在产生它的那一点，造它的人写它**。（`Spot` 那一格后来整格退场：29 处写、
/// 0 处读，见本文件顶上那条照实记。）
///
/// **三个答案各有读者**：`Resident` 与（`Transient` / `Told`）的分界就是"**该收了**"那道闸
/// （`core::due`）；而 `Transient` 与 `Told` 的分界是**静默兜底**（`core::walking`——听令那一台
/// 的沉默是正常的，它的超时归外面那一层）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ending {
    /// 我收它：只有被收才走（两个域 ＋ 七台服务）。
    Resident,
    /// 它自己走：到了自然结尾就退场（客人 / 探针）。
    Transient,
    /// 有人叫它走：终止词从外面来（控制台那一台）。
    Told,
}

/// **这一趟装配本身**（名单里那个目标单元的名字）：写法是 `after: Some(&[…, SCENE])` =
/// "**等这一趟装配走完**再起我"。今天只有 `probe-control` 一家写它：它问的
/// `/svc/sys/control/state` 由装配者在**整表起完之后**才铸、且要等监督那一趟开始才被**服务**
/// ——而"整表起完"不是一个台，图里本来没有它的落点。
///
/// **它是 [`Kind::Target`] 那个单元的名字**（[`SCENE_UNIT`]）：两边写的是同一个词，一处给
/// （静态那份声明的 `identity.name` 就是它）。
///
/// **到点是什么意思**（唯一一处）：[`order_scene`] 把写它的台排到**最后**（同批按名字）；编排域
/// 起完它们之后立刻挂上 control 那一面、进监督那一趟（`system/main.rs` 的相四）。**装配那一趟里
/// 不等它**——[`Assembly::assemble`](crate::system::Assembly::assemble) 逐条边等"那一台答得动"时
/// 跳过它：等一个"这一趟"没有可等的对象，**排到最后就是它的全部保证**。
///
/// **照实记（它为什么从一格布尔变成一个名字）**：那一格从前是 `Relation::after_scene: bool`——
/// `true` 说的就是这句话，而"这一趟"在图上没有落点，于是它只能是一格 flag，两个读者各自把这句话
/// 猜一遍。今天它与别的边同一个写法（`after` 里写名字），而"什么时候算到点"只写在这里。
///
/// **照实记（它从"一个名字"升成"一个单元"）**：上一刀它只是一个名字（[`is_target`] 判的就是这个
/// 名字）。今天它是**名单里的一台**（[`PROGRAMS`] 里有 [`SCENE_UNIT`]，`Kind::Target`）：判据因此
/// 从"名字等于 `SCENE`"变成"**那一台是目标单元**"——名字与类型的区别在那里显出来（`is_target`
/// 不再认识字面量），而"图里的点"这一件事终于有了自己的型。
pub const SCENE: &str = "scene";

/// **这一趟装配本身**——名单里的那个[目标单元](Kind::Target)：**没有身子、不进任何镜像**（宿主
/// 那一侧按 `wanted_by` 与 `kind` 两格把它滤掉），它对这张单的贡献只有一件事：**给"这一趟走完"
/// 一个落点**（[`SCENE`] 那条边指着它）。
///
/// **它为什么住本文件**（照实记）：各台自己那份 `program.rs` 的判据是"声明紧挨着它的身子"
/// （见下面那张 `#[path]` 清单）——而目标单元**没有身子**，故它没有"自己那一份"可言；它属于
/// **图的形状**这一层，与 [`PROGRAMS`] 住同一处。
pub static SCENE_UNIT: Program = Program {
    identity: Identity {
        name: SCENE,
        kind: Kind::Target,
        wanted_by: &[],
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};

/// **这一条边指着的是不是"这一趟自己"**——即那一台是不是[目标单元](Kind::Target)。
///
/// 图里两种点：**要起的服务**（在单上、有身子）与**目标**（不在单上：它只把几条边聚在一起，
/// "这一趟装配走完"就是它的内容）。两个读者判的是同一句话：[`order_scene`] 据它把这一台排到
/// 最后（那一格要到那时才到点），装配那一趟据它跳过那一条边
/// （[`Assembly::assemble`](crate::system::Assembly::assemble)：等一个"这一趟"没有可等的对象）。
///
/// **判据是"类型"不是"名字"**（照实记：这一格与 [`SCENE_UNIT`] 一起改的）：从前这里写的是
/// `name == SCENE` 那个字面量——同一个词在两处出现；今天它问的是**那一台自己的 `kind`**。
pub fn is_target(name: &str) -> bool {
    let mut i = 0;
    while i < PROGRAMS.len() {
        if PROGRAMS[i].name() == name {
            return matches!(PROGRAMS[i].kind(), Kind::Target);
        }
        i += 1;
    }
    false
}

/// **装配关系**：编排域把它接进来时那几条边。
///
/// **这一块只有装配者读**：依赖 / 存在信号 / 结束方式 / 身份——都是
/// "这一台与那一台之间有一条什么边"，与它自己是谁（[`Identity`]）、起手要什么（[`Demand`]）分开。
///
/// **照实记（`holds_tree` 那一格退场：运行期的事实不该写进声明）**：那一格答"我是不是持树者"，
/// 而"是"这件事**要等它起来之后才成立**（它起手把提示之路交给生我者）——声明里写一句 `true`
/// 只是**断言**：断言与实情分家的那一天，没有一处读得出来哪一边对。今天判据是**那一枚孔**：
/// 本域表里有没有一枚**挂在它名下的 `TIP_MARK` 孔**（
/// [`Tree::holds`](crate::system::operator::bridge::Tree::holds)，只看不铸），判它的那一手是
/// [`hold`](crate::system::operator::bridge::hold)。**代价照实说**：从前"持树者没交出提示之路"
/// 报在**它**头上（`operator:tip`），今天报在**第一个要树的客人**头上（`Tree::attach` 的
/// `no tree yet`）——同一景里读得出来，但那一条读数换了个人。
///
/// **照实记（`eyes` 那一格退场：谁是谁由运行期认，不由声明点名）**：那一格从前答"这一台是持树者
/// 的哪一双眼睛"（名册 / 盟册），装配者据此把"谁是名册"记进协调帧推给持树者。而**那件事本来就有
/// 凭据**：名册与盟册各自把门牌交给持树者（`PrincipalGrant::Ask` / `CoalitionGrant::Ask`），
/// 而那两个记号是**协议里各族自己的常量**、各只有一家生产者 ⇒ **持树者按记号就认得出**，用不着谁
/// 告诉它"那一位是几号"。于是这一格连带**整条协调帧**（`Tip::Coord` ＋ `env::wire::Eyes`）一起
/// 退场；本域这一侧只剩一件真事：**名册那一位要认下面 ＋ 补绑自己与树**（
/// [`adopt_roster`](crate::system::principal::bridge::adopt_roster)，判据是名册自己交上来的那一枚
/// `Grant::Set`）。
///
/// **照实记（`operator` 那一格退场：推得出来的事不该再写一遍）**：那一格答"接不接持树者那棵树"，
/// 而**那件事已经写在 [`Relation::after`] 里了**——树就是持树者那一本目录（`find` 回入口、`land`
/// 落自己那几格），故"要用树的东西"与"要问 `operator` 那一族"是同一件事。**量过**（35 份声明逐
/// 份核）：写 `operator: true` 的正是 `after` 含 `"operator"` 的那 **21** 台，写 `false` 的 **0** 台；
/// 余下 14 份里 12 份**没写** `after`（默认 `None`）、2 份写 `after: Some(&[])`（`passer` 与持树者
/// 自己）——两处都空。今天那一手住在树那一轴自己那里
/// （[`bridge::attach_client`](crate::system::operator::bridge::attach_client)），判据从这一台
/// 自己的 `after` 推——**同一句话只有一处**。
#[derive(Clone, Copy)]
pub struct Relation {
    /// **`After=`**（systemd 同名那一格）：**这一台要等哪几位答得动**——起手那一趟的次序由它算
    /// 出来（**不再手排位次**）。
    ///
    /// 一条边 = "我起手的一个动作要求它已经**答得动**"；成立的凭据是那一台自己交回的那枚孔
    /// （[`Setup::Ready`] / [`Setup::Machine`] 的第二条）。`None` = **不在这一趟装配单上**
    /// （引导镜像 / 编排域自己 / 压测台）——与旧 `order: Option<u8>` 的 `None` **同义**。
    ///
    /// **照实记（它原名 `deps`，这一刀改成 `after`）**：`deps` 说的是"依赖"——一个**关系**词，
    /// 什么都能算依赖（谁等谁、谁给谁东西、谁替谁保管）；而这一格**只答一件事**：**等谁**。
    /// 于是它的读者只有一处口径（装配者算次序），而它今天要治的病（`rtc`/`uart` 靠**位次**问路由者、
    /// `deps` 里却没写 `router`）正是"关系词把'等谁'含糊掉了"的后果 ⇒ 借 systemd 的 `After=` 说死：
    /// **这一格只说次序，不说别的**。
    ///
    /// **它只说得动"答得动"，说不动"死没死"**（照实记）：三条边写不出来——整机物料是**装配者递的**
    /// （那是 [`Setup::Machine`] 那一格）；`probe-owner` 等的是"`/svc/lease` 的主人**死掉**"，
    /// 那是"等它**不在**"——图只表达"要它在"，故那一等留在它自己那圈重试里。
    ///
    /// **三条边在这里说不出口，故不写**（照实记）：板不是这一列的台（不成边）；整机物料是
    /// **装配者递的**（那是 [`Setup::Machine`] 那一格）；`probe-owner` 等的是"`/svc/lease`
    /// 的主人**死掉**"——图只表达"要它答得动"，"等它死"仍在它自己那圈重试里。
    pub after: Option<&'static [&'static str]>,
    /// **`Restart=`**（systemd 同名那一格）：**这一台的寿命由谁定**——`None` = 没声明；
    /// **由编排域起的台必须写**（`Control::enlist` 当场拒）。取值是 [`Ending`] 那一型
    /// （`Resident` = 常驻、`Told` = 被叫停），故这一格的名字说的是**声明哪一个键**，
    /// 值那一型说的是**哪一种寿命**。
    ///
    /// **照实记（它原名 `ending`，这一刀改成 `restart`）**：`ending` 说的是"它怎么结束"——一个
    /// **描述**词；而这一格在 systemd 那一侧是**一条声明**（`Restart=`），且它的读者只有收场那一相
    /// （编排域据此判"它没走的时候等不等"）。借同名那一格说死：**这一格是"编排域怎么对待它的
    /// 寿命"**，不是"它自己怎么结束"。
    ///
    /// **收场那一相只读它**（[`crate::system::control::core`] 那三具判定）——"它没走的时候编排域
    /// 等不等"全在这一格上。**照实记（它从前与 `presence` 成对）**：那一格答"它死了谁知道"，
    /// 而它随板那一族退场了（见 [`Relation`] 底下那条照实记）⇒ 今天这一格是**孤零零**的一格，
    /// 而这正是它该在的样子：**寿命由谁定**与"死讯怎么来"是两件事，后者今天由监督那一趟的
    /// 表侧那一扫兜着。
    pub restart: Option<Ending>,
    // **照实记（`presence` 那一格退场：它和它的对偶一起退）**：它答"要不要存在信号"——而那个
    // 信号（一条 `gone-<名字>` 的道）已经整片退场（死改由监督那一趟的表侧扫认，见
    // `system::control::supervise` 的 `Watch::new`）。它剩下的唯一读者是**装配者接板那一手**，
    // 而那一手与**客侧开板会话**是一对握手的两头（`presence: true` 的台必须自己开一条
    // `board::BERTH` 会话，否则装配当场报 `board:claim`）⇒ 两头一起撤。
    //
    // **原文留档（它这一格的两次改动的全部理由）**：
    //
    // 照实记（改名那一刀）：这一格原先叫"上不上板"，问的是 `Board` 那一台；而板那一侧已经退成
    // **一枚死信号传感器**——"上板"这个说法只剩历史。故按它真正答的那句话改名：**要不要存在信号**。
    //
    // 照实记（"存在信号"那一半退场：它今天答的是"这一台开不开板会话"）：量过"死由表侧那一扫独自
    // 认"之后，装配那一侧**不再铸道**——`gone-<名字>` 那一族退场，本格的那个读者（`Watch::of`）
    // 跟着没了。那时它只剩**一个读者**（`board::bridge::attach_client`：问"这一台要不要一个板位"），
    // 而板那一族今天整族退场，这一格跟着退。
    /// 放行前给不给**身份**（`false` = 没绑身份，撞门该被拒——负证客人就是靠它）。**默认给**：
    /// 只有负证那一台明写 `false`（照实记见 [`Relation::DEFAULT`]）。
    pub bind: bool,
}

/// **需求**：实例化它要多做的那几手、它死在装配哪一步的号。
///
/// **这一块只有装配者读**，两格答的是同一句话的两面："把它弄起来要动用什么"。
#[derive(Clone, Copy)]
pub struct Demand {
    /// 死在装配哪一步的号。`after` 为 `None` 的那几台不读这一格（写 [`env::EXIT_OK`]）。
    ///
    /// **照实记（它为什么与 `supply` 同块）**：`died` 是**这一台 `supply` 走不通时**的读数——
    /// 与起手那几手是同一件事的两面，故同块。
    pub died: Died,
    /// **`Type=notify`**（systemd 同名那一格）：**它起来之后要说一句"答得动了"**——装配者在
    /// 放行之后按这一格开通道、等那一句（[`Setup::Ready`]）或递物料（[`Setup::Machine`]）。
    ///
    /// **照实记（它原名 `setup`，这一刀改成 `supply`）**：`setup` 说的是"它起手要做的那几手"
    /// （一个**动作**词，读起来像"它自己装自己"）；而这一格在装配那一侧说的是**装配者要对它做的
    /// 那几手**（开一条通道等它报"答得动"／把整机物料递过去）⇒ 借 systemd 同名那一格说死：
    /// **它给我什么**（`Type=notify`：它报"我能答了"，我据此往下起别人）。
    ///
    /// **照实记（"推得出来"这件事我认错过两次，第三次才对——逐份量，不按文件数）**：
    /// · 第 21 轮（施工图）：我写"8 处 `Ready` **推得出来**"（"被 `after` 点名的台必须交凭据"）。
    /// · 第 24 轮：我按**文件**数了一遍，得出"两套集合不重合"（还说 `probe-rule` 被点名却没写）
    ///   ⇒ 写下"推不出来"这条证伪。**那一次是错的**：`decl/harness.rs` 里 23 份声明，按文件数
    ///   只会得到"这一份写了"，认不出是**哪一台**。
    /// · 第 27 轮**逐份量**（`pub static X` 与它那块里的 `supply:` 对齐）：
    ///   写 `Setup::Ready` 的**恰好 7 份**：`uart` / `router` / `rtc` / `operator` / `principal` /
    ///   `coalition` / **`PROBE_RULE`**；被 `after` 点名的**8 台**：上面那 7 台 ＋ **`hub`**。
    ///   ⇒ **两套集合差且只差 `hub`**，而 `hub` 的"答得动"就写在 [`Setup::Machine`] 的 `ready`
    ///   那一格上 ⇒ **按"交不交'答得动'凭据"这件事说，两套完全重合**。
    /// **故正确的结论是**：`Ready` 那一半**确实推得出来**（"谁被 `after` 点名，谁就必须交"）；
    /// **推不出来的是 `Machine` 那一半**（整机物料 ＋ 它那条通道的名字只有设备账那一台有）——
    /// 所以这一格今天仍是**声明**，而"把 `Ready` 那一支收掉、只留 `Machine`"是**可以做的下一刀**
    /// （要做就得让装配者按 `after` 反查"谁必须交"，并且**先证**两套集合在每一景里都重合）。
    pub supply: &'static [Setup],
}

// ── 三块各自的"什么都没声明"那一形（回炉那一刀；照实记）──────────────────
//
// **它是什么**：一台程序**最少要说的话**。那 34 份声明从前把 14 格一格不落地写一遍（实测
// **476 次写**），而其中大半是每一台都一样的那几格。今天各台只写**与这一形不同的格**，其余用
// 结构更新语法（`..Identity::DEFAULT`）收掉：实测 **476 → 231 次写（−51%）**，那 34 份的
// 内部行数 **687 → 462**。
//
// **这两套值都量过，选的是第二套**（照实记）：按**多数值**取（当时那几格里的 `presence: false`
// / `operator: true` / `Spot::Rig` …）能把那 34 台压到 **212** 次写，比下面这一套少 19 次——
// 而那一套的 `DEFAULT` 说的是一台**压测台**：新加一台的人会**静默继承**"压测台"这个身份。
// 中性这一套买的是"`DEFAULT` 这个词说得通"：**进一段 `"root"` 景、不由编排域起、不上板 /
// 不绑身份（`after: None` ⇒ 也不接树）、没有起手那几手**。
// （那几格后来各自退了场：`operator` / `holds_tree` / `after_scene` / `eyes` 四刀见 [`Relation`]
// 与 [`SCENE`] 的头注，`Spot` 见本文件顶上那条，`origin` 见 [`Demand`] 底下那条。）
//
// **为什么是关联常量而不是 `Default` trait**：那 34 份是 `pub static`，初始化器**必须是常量
// 表达式**，而 `Default::default()` 不是 `const`。
//
// **`name: ""` 是占位**（零字节的名字非法）：每一台都必须自己写那一格，故它一次都没省下。

impl Identity {
    /// **什么都没声明的那一形**（中性，不是多数值——见上面那一节）。
    pub const DEFAULT: Identity = Identity {
        name: "",
        kind: Kind::Service,
        space: ProgramKind::User,
        wanted_by: &["root"],
        entry: &[],
    };
}

impl Relation {
    /// **什么都没声明的那一形**：不在装配单上（`after: None`）⇒ 不上板，也**没说自己怎么结束**
    /// （`restart: None`——由编排域起的台不写它，装配那一趟当场拒）。
    ///
    /// **照实记（`bind` 这一格取的是"正常那一档"，不是中性那一档）**：这一形其余几格都是中性值，
    /// 唯独 `bind: true`。**量过**（A 面逐份核）：35 份里 **22 份写 `true`、0 份写 `false`**，而
    /// **唯一的那个 `false` 是靠"不写"表达的**——`probe-denied` 吃这一形，而它自己的注释里写着
    /// `bind: false`、声明里却没有那一行。**"用一个不写的地方表达唯一一个反例"正是这一层最贵的
    /// 形状**（新加一台的人会**静默继承**那个反例，而装配那一趟不会报错）⇒ 这一格翻成**正常那一
    /// 档**，反例**明写**。余下 13 份（`after: None` 的那几台）不吃这一格：装配那一趟只为**在单上
    /// 的台**叫这一手。
    pub const DEFAULT: Relation = Relation {
        after: None,
        restart: None,
        bind: true,
    };
}

impl Demand {
    /// **什么都没声明的那一形**：没有起手那几手、号报"正常退场"。
    pub const DEFAULT: Demand = Demand {
        died: env::EXIT_OK,
        supply: &[],
    };
}

// **照实记（`Origin` 那一格退场：0 个选择者）**：它答"这一台的身子从哪本账里取"（`Initrd` /
// `Storage` 两档）——而 **35 份声明里没有一处写过它**（全走 [`Demand::DEFAULT`] 那一档），且
// `Storage` 那一档的"实现"只有一个出口（`system::source::Error::NoSource`：那一台——盘 / 文件
// 系统——**不存在**）。⇒ 这一维**只有一个值**：写成枚举等于把"以后会有"写在今天，而在装配那一趟
// 里多出来的是一条**构造上到不了**的分支（唯一消费者是 [`Control::spawn`] 那一行）。
//
// **它要回来的话，回来的是一本账**（[`crate::system::source`] 那种读面），不是声明上一个变体：
// 打包那一侧按 `wanted_by` / `entry` 决定镜像进哪本账，装配那一侧只问"那一段字节在哪"。

/// 实例化一台要多做的一手。
///
/// **它不负责 start**：两种都在装配那一趟里按次序落到具体那几手上（见
/// [`crate::system::control`] 的 `connect_all` / `enroll`）。
///
/// **"怎么算它起来了"由这一格推**（有通道 ⇒ `Announce::Channel`）——故不给它另立一格。
/// **装配者等的那一枚记号**（[`Setup::Ready`] 那一条）：驱动那三台落完面之后铸一枚刻它的孔
/// 交给装配者。**记号可以共用**——装配者按 `owner ＋ 记号` 两格认（`Endpoint::claim`），
/// 一台一份，不会串。
pub const READY: &str = "ready";

#[derive(Clone, Copy)]
pub enum Setup {
    /// **答得动了**：这一台交回一枚刻 `READY` 的孔 ⇒ "**我这一面已经在树上、答得动**"。
    ///
    /// **照实记（"记号"那个参数退场：9 处里 8 处写的是同一个常量）**：它从前是
    /// `Ready(&'static str)`，而**全仓 8 处写的是同一个** [`READY`]（第 9 处是 [`Setup::Machine`]，
    /// 它自带两个名字）。那个参数因此是**同一句话的第二处**——一件"这一段字节"上不存在的选择。
    /// 收掉之后这个记号只有一处（[`READY`]），`channel()` 直接答它。
    ///
    /// **它在起手哪一步交，就是这一格的全部内容**：设备账那条 `Machine.ready` 在**落完每一台
    /// 机器的格**之后交；驱动那三台（`router` / `uart` / `rtc`）在 `ctx.plate(..)` **之后**交
    /// ——落面本来就是它们起手的最后一步（"**牌子的次序 = 什么时候答得了**"那条照实记写在
    /// `driver/uart/desk.rs` 与各驱动那一侧）。
    ///
    /// **为什么非有这一格（量出来的）**：装配者原先判"它起来了"只有一格——**"放行之后只要还
    /// 活着"**（`control/service.rs::ready` 的 `Announce::None` 那一支：全表 25 台里只有设备账
    /// 声明了 `setup`）。而"面"落在起手的最后一步、装配者那时早已往下起了好几位 ⇒ 后来者
    /// （`lodger` 问 `router`、`sleeper` 问 `rtc`、盟册问名册）问的都是**还没答得动的机器**，
    /// 手里只有自己那 1 s 有界重试 ⇒ 慢一点的世界里就是一片 `no /svc*`。
    /// ⇒ **次序要治的不是"排第几"，是把"起来了"的含义从"活着"改成"答得动"。**
    Ready,
    /// **整机物料**：这一台起手要**这台机器的全部可领之物**（设备树本体 / 门铃 / 每一台设备
    /// 那一段区）。
    ///
    /// 与 [`Setup::Ready`] 是**同一手 ＋ 一件事**：放行前照样 `connect`（它交回那一枚照样是
    /// "我起来了"），放行之后装配者多走一趟——**照 [`crate::system::machine::Machine::devices`]
    /// 枚举全机**、逐段向引导域领、再把那一段记录从这条通道推给它。
    ///
    /// **为什么这条通道由装配者填、而不是收方自己去领**（照实记）：与引导域搭那条问答路的
    /// 泊位（`protocol::system::supply` 的 `BOOT`）**在装配者手里**——它是唯一持它的那一方。
    /// 收方（设备账那一台）拿不到它，也拿不到"我该领哪几样"（那要读一遍设备树，而树本身也在
    /// 那一份物料里）。⇒ 领那一段只能发生在装配者这一侧；**这一段记录**因此是"机器 → 那一台"
    /// 的单向一次交接。
    ///
    /// **今天只有一台要它**（设备账 `hub`），而这一格是**声明**：加第二台时不用动装配者一个字。
    ///
    /// **它是两条通道，不是一条**（照实记，量出来的）：第一条收物料（`load`），第二条报
    /// "**我把这一台机器的格都落完了**"（`ready`——它由收方在起手末尾铸一枚孔交回来）。
    ///
    /// 为什么非有第二条：收方的起手很长（读一遍树、逐类立盟、逐类逐台落格 ＋ 每格查回来验一遍
    /// ——实测**这台机器上最慢的一次起手**），而**放行之后装配者就往下起下一位了**。只凭第一条
    /// 通道，"我起来了"说的是"我刚收到物料"——那一刻它一颗设备格都还没落，后面那几位客人
    /// （驱动）来认领只会撞空。两条通道分开，`ready` 那一格才说得清"**能答了**"。
    ///
    /// **两条都在 [`Setup::channels`] 里**：装配者按同一只手把它们都 `connect` 上，`ready`
    /// 那条由 `service::ready` 一起等（"逐条凑齐了才算起来"那条口径原样成立）。
    Machine {
        /// 收物料那条通道的名字。
        load: &'static str,
        /// **"我起完了"那条通道**的名字（收方在起手末尾铸一枚刻它的孔）。
        ready: &'static str,
    },
}

impl Setup {
    /// 这一格要开的**第一条**通道（恒有一条）：`Channel` 那一格就是它，`Machine` 那一格是
    /// **收物料**那条。
    pub const fn channel(&self) -> &'static str {
        match self {
            Setup::Ready => READY,
            Setup::Machine { load, .. } => load,
        }
    }

    /// **还有第二条吗**——`Machine` 那一格多一条（"**我起完了**"那条，见它自己的注）；
    /// `None` = 这一格只开一条通道。
    pub const fn ready(&self) -> Option<&'static str> {
        match self {
            Setup::Ready => None,
            Setup::Machine { ready, .. } => Some(ready),
        }
    }

    /// 这一格是不是"整机物料"那一类（装配者据此在放行之后多走一趟入册）。
    pub const fn machine(&self) -> bool {
        matches!(self, Setup::Machine { .. })
    }
}

// ── 每台自己的声明 ───────────────────────────────────────────────────
//
// **它们不在这份文件的自然模块树里**：那些目录（`driver/uart/`、`system/operator/`…）都拖着
// runtime / protocol 代码，`crates/image` 进不去。故只由 [`PROGRAMS`] 这一处按 `#[path]` 拉
// 进来一次——**唯一的声明点**。

#[path = "../user/canonical/program.rs"]
pub mod canonical;
#[path = "../system/coalition/program.rs"]
pub mod coalition;
/// harness 那 23 台（**测具**）：它们的身子住隔壁那个 crate，而其中 13 台**由编排域起**
/// ——编排域要按 `order` / 存在信号 / `bind` / `died` 起它们，故声明必须由本 crate 编译。
/// `harness` 依赖 `programs`，反向不可能。故这一族的声明住这里（一份，不拆 23 份：
/// "紧挨着身子"对身子不在本 crate 的那几台本来就不成立，不假装）。
#[path = "../decl/harness.rs"]
pub mod harness;
#[path = "../system/hub/program.rs"]
pub mod hub;
#[path = "../system/operator/program.rs"]
pub mod operator;
#[path = "../system/principal/program.rs"]
pub mod principal;
#[path = "../root/program.rs"]
pub mod root;
#[path = "../driver/router/program.rs"]
pub mod router;
#[path = "../driver/rtc/program.rs"]
pub mod rtc;
#[path = "../system/program.rs"]
pub mod system;
#[path = "../driver/uart/program.rs"]
pub mod uart;

/// **装配表**：镜像里可能有的全部程序。**次序是硬事实**——它就是装载次序（`ROOT_OFFSET`
/// 按位次算），且各景按 [`Program::wanted_by`] 过滤 ⇒ 加一台要想清楚放哪。
///
/// **本表只有引用**：每一台的声明都在它自己那份 `program.rs` 里，这里不再写第二遍。
///
/// **本表一行一台，`rustfmt` 请绕开**：默认那套会把每台摊成十几行，于是"哪几台进哪张镜像"
/// 就没法一眼扫完——而这张表**就是**给人扫的。
#[rustfmt::skip]
pub const PROGRAMS: &[&Program] = &[
    &root::PROGRAM,
    // 四枚服务（持树者 / 名册 / 盟册 / 设备账）：各自一个 bin、一个域，与其他每一台同一条 `mint` 路。
    &operator::PROGRAM,
    &principal::PROGRAM,
    &coalition::PROGRAM,
    &hub::PROGRAM,
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
    // 控制面那位真客人（`/svc/sys/control/state` 那一格）：**排在 `canonical` 之前**，见它自己那份声明。
    &harness::PROBE_CONTROL,
    // 操作面那一族（`/svc/sys/operator/{part,land,…}`）：**两位一对**——`gate` 拿控制面会话把七格
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
    // **这一趟装配本身**（[`SCENE_UNIT`]）：一个[目标单元](Kind::Target)——没有身子、不进任何
    // 镜像（宿主那一侧按 `wanted_by` 与 `kind` 两格滤掉），它在这张表里只为"这一趟走完"给一个落点。
    &SCENE_UNIT,
];

/// 清单条数上界与注册表条数必须相容（见 [`env::manifest::MAX_PROGRAMS`] 的头注）。
///
/// **这一条就是从前那个"数出来的数"的替身**：加一台超过上界 ⇒ 当场编不过，不可能静默卡住。
const _: () = assert!(PROGRAMS.len() <= env::manifest::MAX_PROGRAMS);

// ── 这一张单自己算不了的那一件事：**次序**（两个读者共用这一份）──────────────

/// 图上说不通的那三种——每一种都报出**名字**。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DepsFail {
    /// 一条边指着本单里没有的台（名字）。
    Unknown(&'static str),
    /// 被指着的那一台**没有凭据**（`demand.supply` 空 ⇒ 它交不出"我答得动"）。
    NoEvidence(&'static str),
    /// 取不出可排的台 ⇒ 环（名字 = 卡住的那一个）。
    Cycle(&'static str),
}

/// **按 `after` 把这一张单排成次序**（拓扑，原地重排）：每条边都在前面；等 [`SCENE`]（这一趟走完）
/// 的排**最后**。
/// 同一批按**名字**排（与声明次序无关，可复现）。图上那三种说不通当场挑出来（[`DepsFail`]）。
///
/// **两个读者共用这一份**（照实记）：宿主那一侧打包时校验（`crates/image`，报得出名字），
/// 引导期那一趟排次序（`system::assemble`）。故它**不许分配**——本文件是宿主安全的
/// （只许引 `env`，`crates/image` 用 `#[path]` 文本包含它），只用切片与定长栈。
///
/// **它不解释任何一台的字段**：只读 [`Relation::after`] 那一格（[`SCENE`] 只是其中一个名字）。
pub fn order_scene(list: &mut [&'static Program]) -> Result<(), DepsFail> {
    // 一、每条边都要落得下：指得到本单里的台，且那一台说得出"我答得动"。
    //     **[`SCENE`] 那一条除外**：它指的是这一趟自己，不是本单里的台，也没有"答得动"可言
    //     （见 [`SCENE`] 的头注）。
    let mut i = 0;
    while i < list.len() {
        if let Some(deps) = list[i].relation.after {
            let mut d = 0;
            while d < deps.len() {
                let name = deps[d];
                // **[目标单元](Kind::Target)那一条除外**：它指的是这一趟自己，不是本单里的台，
                // 也没有"答得动"可言（见 [`SCENE_UNIT`] 与 [`is_target`]）。
                if !is_target(name) {
                    match find(list, name) {
                        None => return Err(DepsFail::Unknown(name)),
                        Some(target) if target.demand.supply.is_empty() => {
                            return Err(DepsFail::NoEvidence(name))
                        }
                        Some(_) => {}
                    }
                }
                d += 1;
            }
        }
        i += 1;
    }
    // 二、拓扑：一轮取"前置都排好了"的那一个；同批挑名字最小的（可复现）。
    let n = list.len();
    let mut placed = 0usize;
    while placed < n {
        let mut pick: Option<usize> = None;
        let mut i = placed;
        while i < n {
            if !waits_scene(list[i]) && ready(list, i, placed) {
                match pick {
                    Some(best) if list[best].name() <= list[i].name() => {}
                    _ => pick = Some(i),
                }
            }
            i += 1;
        }
        let Some(i) = pick else { break };
        list.swap(placed, i);
        placed += 1;
    }
    // 三、收尾：剩下的必须全是等[目标单元](Kind::Target)的（不是 ⇒ 环）；它们同批按名字。
    let mut i = placed;
    while i < n {
        if !waits_scene(list[i]) {
            return Err(DepsFail::Cycle(list[i].name()));
        }
        i += 1;
    }
    let mut i = placed;
    while i < n {
        let mut pick = i;
        let mut j = i + 1;
        while j < n {
            if list[j].name() < list[pick].name() {
                pick = j;
            }
            j += 1;
        }
        list.swap(i, pick);
        i += 1;
    }
    Ok(())
}

/// **这一台等的是"这一趟走完"吗**——`after` 里有一条边指着[目标单元](Kind::Target)就是。
///
/// 它有两个读者，判的是同一句话：[`order_scene`] 据它把这一台排到最后（那一格要到那时才到点），
/// 而装配那一趟据它跳过那一条边（`Assembly::assemble`：等一个"这一趟"没有可等的对象）。
fn waits_scene(program: &Program) -> bool {
    program
        .relation
        .after
        .is_some_and(|deps| deps.iter().any(|name| is_target(name)))
}

/// 这一台的**边都排好了吗**（`list[..placed]` 里找得到每一条边指着的那一台）。
fn ready(list: &[&'static Program], i: usize, placed: usize) -> bool {
    let Some(deps) = list[i].relation.after else {
        return true;
    };
    let mut d = 0;
    while d < deps.len() {
        let mut found = false;
        let mut j = 0;
        while j < placed {
            if list[j].name() == deps[d] {
                found = true;
                break;
            }
            j += 1;
        }
        if !found {
            return false;
        }
        d += 1;
    }
    true
}

/// 本单里按名字找那一台（**只查不比存** ⇒ 借 `&str`）。
fn find<'a>(list: &[&'a Program], name: &str) -> Option<&'a Program> {
    let mut i = 0;
    while i < list.len() {
        if list[i].name() == name {
            return Some(list[i]);
        }
        i += 1;
    }
    None
}
