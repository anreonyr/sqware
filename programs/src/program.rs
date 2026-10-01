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
    /// **常驻服务**：三台驱动 ＋ 四枚服务（持树者 / 名册 / 盟册 / 设备账）。
    Service,
    /// **控制台那一台**（`canonical`）：本域扮**终端那一侧的行规程**（ECHO / ERASE / KILL / EOF）；
    /// 产品镜像里排**最后**——**排最后不再承重**（照实记）：从前编排域等它退场才收场，
    /// 故"它是最后一位"是一条硬前提；这一刀之后收场的判据是账（`Control::{due, done}`），
    /// 位次只管"先起谁"。它**听令才走**（终止词从外面来）——那一格住在
    /// [`Relation::ending`]（`Ending::Told`），住它自己那份 `program.rs` 里。
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
///   Relation   它跟谁有边（依赖 / 排最后 / 存在信号 / 身份 / 眼睛）
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

    /// **它在不在这一趟装配单上**（`relation.deps: Some`）。
    ///
    /// **宿主那一侧的第二组窄面**（照实记：本表原先只开四扇门，都取 [`Identity`]）：打包那一趟
    /// 仍只读前四样，而**校验那一趟**（[`order_scene`] 的调用点）要按这一格滤出"由编排域起的
    /// 那些台"——图的内部（`deps` 的内容、`after_scene`）一律由本文件那两具读，宿主不碰。
    pub fn listed(&self) -> bool {
        self.relation.deps.is_some()
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

/// **谁结束它**——这一台的寿命由谁定。
///
/// **照实记（为什么不按 `Spot` 分类）**：`Spot` 答的是"这一台是什么"（角色），而它自己的头注
/// 就写着"**不许拿它当'装不装'用**"；按角色反推"会不会自己走"与 `Start::code` 那三组硬编码是
/// 同一类错——那一刀**量出 8 处假读数**（名册死在树上报的是持树者…）。故按那条纪律办：
/// **事实放在产生它的那一点，造它的人写它**。
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

/// **装配关系**：编排域把它接进来时那几条边。
///
/// **这一块只有装配者读**：依赖 / 排最后 / 存在信号 / 结束方式 / 身份 / 眼睛——都是
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
/// **照实记（`operator` 那一格退场：推得出来的事不该再写一遍）**：那一格答"接不接持树者那棵树"，
/// 而**那件事已经写在 [`Relation::deps`] 里了**——树就是持树者那一本目录（`find` 回入口、`land`
/// 落自己那几格），故"要用树的东西"与"要问 `operator` 那一族"是同一件事。**量过**（35 份声明逐
/// 份核）：写 `operator: true` 的正是 `deps` 含 `"operator"` 的那 **21** 台，写 `false` 的 **0** 台；
/// 余下 14 份里 12 份**没写** `deps`（默认 `None`）、2 份写 `deps: Some(&[])`（`passer` 与持树者
/// 自己）——两处都空。今天那一手住在树那一轴自己那里
/// （[`bridge::attach_client`](crate::system::operator::bridge::attach_client)），判据从这一台
/// 自己的 `deps` 推——**同一句话只有一处**。
#[derive(Clone, Copy)]
pub struct Relation {
    /// **我起手要问谁**——装配那一趟的次序由它算出来（**不再手排位次**）。
    ///
    /// 一条边 = "我起手的一个动作要求它已经**答得动**"；成立的凭据是那一台自己交回的那枚孔
    /// （[`Setup::Ready`] / [`Setup::Machine`] 的第二条）。`None` = **不在这一趟装配单上**
    /// （引导镜像 / 编排域自己 / 压测台）——与旧 `order: Option<u8>` 的 `None` **同义**。
    ///
    /// **三条边在这里说不出口，故不写**（照实记）：板不是这一列的台（不成边）；整机物料是
    /// **装配者递的**（那是 [`Setup::Machine`] 那一格）；`probe-owner` 等的是"`/svc/lease`
    /// 的主人**死掉**"——图只表达"要它答得动"，"等它死"仍在它自己那圈重试里。
    pub deps: Option<&'static [&'static str]>,
    /// **等装配那一趟走完**——只有"要问的面由装配那一趟末尾才立起来"的台才写。
    ///
    /// **为什么它不是位次**（照实记）：今天只有 `probe-control` 一处——它问的
    /// `/svc/sys/control/state` 由装配者在**整表起完之后**才铸、且要等监督那一趟开始才被**服务**；
    /// 那**不是一个台**，图里没有这条边的落点。旧写法靠 `order: Some(21)` 把它压到末尾；次序一旦
    /// 由**边**算，它可能被排到第二（窗口反而变大）。故把"我要的那个状态"写成一条**边指向隐式
    /// 节点**：装配那一趟走完。图里它排最后，且没有可等的边。
    pub after_scene: bool,
    /// **谁结束它**——`None` = 没声明；**由编排域起的台必须写**（`Control::enlist` 当场拒）。
    ///
    /// **它与 [`Relation::presence`] 成对**：那一格答"它死了谁知道"，这一格答"它没走的时候
    /// 编排域等不等"。收场那一相只读它（[`crate::system::control::core`] 那三具判定）。
    pub ending: Option<Ending>,
    /// **要不要存在信号**（原 `board`）。
    ///
    /// **照实记（改名那一刀）**：这一格原先叫"上不上板"，问的是 `Board` 那一台；而板那一侧已经
    /// 退成**一枚死信号传感器**（"它没了"这件事要有人报到编排域，见
    /// [`system::board`](crate::system::board)）——"上板"这个说法只剩历史。故按它真正答的那句话
    /// 改名：**要不要存在信号**。铸道与接板都只读这一格（`Watch::of` 铸道、`Bridge::attach` 接上）。
    pub presence: bool,
    /// 放行前给不给**身份**（`false` = 没绑身份，撞门该被拒——负证客人就是靠它）。
    pub bind: bool,
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
    /// 死在装配哪一步的号。`deps` 为 `None` 的那几台不读这一格（写 [`env::EXIT_OK`]）。
    ///
    /// **照实记（它为什么与 `setup` 同块）**：`died` 是**这一台 `setup` 走不通时**的读数——
    /// 与起手那几手是同一件事的两面，故同块。
    pub died: Died,
    /// 实例化它要多做的那几手（资源 / 通信）。
    pub setup: &'static [Setup],
}

// ── 三块各自的"什么都没声明"那一形（回炉那一刀；照实记）──────────────────
//
// **它是什么**：一台程序**最少要说的话**。那 34 份声明从前把 14 格一格不落地写一遍（实测
// **476 次写**），而其中大半是每一台都一样的那几格。今天各台只写**与这一形不同的格**，其余用
// 结构更新语法（`..Identity::DEFAULT`）收掉：实测 **476 → 231 次写（−51%）**，那 34 份的
// 内部行数 **687 → 462**。
//
// **这两套值都量过，选的是第二套**（照实记）：按**多数值**取（`Spot::Rig` / `presence: false`
// / `operator: true`（那一格后来退了场——它推得出来，见 [`Relation`] 的头注）…）能把那 34 台
// 压到 **212** 次写，比下面这一套少 19 次——而那一套的
// `DEFAULT` 说的是"**一台名叫 `guest` 的压测台**"：新加一台的人会**静默继承**"压测台"这个
// 角色。中性这一套买的是"`DEFAULT` 这个词说得通"：**角色是常驻服务、进一段 `"root"` 景、
// 不由编排域起、不上板 / 不绑身份 / 不持树 / 没有眼睛（`deps: None` ⇒ 也不接树）、身子从
// initrd 来、没有起手那几手**。
//
// **为什么是关联常量而不是 `Default` trait**：那 34 份是 `pub static`，初始化器**必须是常量
// 表达式**，而 `Default::default()` 不是 `const`。
//
// **`name: ""` 是占位**（零字节的名字非法）：每一台都必须自己写那一格，故它一次都没省下。

impl Identity {
    /// **什么都没声明的那一形**（中性，不是多数值——见上面那一节）。
    pub const DEFAULT: Identity = Identity {
        name: "",
        kind: ProgramKind::User,
        spot: Spot::Service,
        scenes: &["root"],
        entry: &[],
    };
}

impl Relation {
    /// **什么都没声明的那一形**：不在装配单上（`deps: None`）⇒ 不上板、不上树、不绑身份、不持树、
    /// 没有眼睛，也**没说自己怎么结束**（`ending: None`——由编排域起的台不写它，装配那一趟当场拒）。
    pub const DEFAULT: Relation = Relation {
        deps: None,
        after_scene: false,
        ending: None,
        presence: false,
        bind: false,
        eyes: None,
    };
}

impl Demand {
    /// **什么都没声明的那一形**：身子从 initrd 来、没有起手那几手、号报"正常退场"。
    pub const DEFAULT: Demand = Demand {
        origin: Origin::Initrd,
        died: env::EXIT_OK,
        setup: &[],
    };
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
    /// **答得动了**：这一台交回一枚刻着这个记号的孔 ⇒ "**我这一面已经在树上、答得动**"。
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
    Ready(&'static str),
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
            Setup::Ready(ch) => ch,
            Setup::Machine { load, .. } => load,
        }
    }

    /// **还有第二条吗**——`Machine` 那一格多一条（"**我起完了**"那条，见它自己的注）；
    /// `None` = 这一格只开一条通道。
    pub const fn ready(&self) -> Option<&'static str> {
        match self {
            Setup::Ready(_) => None,
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

#[path = "user/canonical/program.rs"]
pub mod canonical;
#[path = "system/coalition/program.rs"]
pub mod coalition;
/// harness 那 23 台（**测具**）：它们的身子住隔壁那个 crate，而其中 13 台**由编排域起**
/// ——编排域要按 `order` / 存在信号 / `bind` / `died` 起它们，故声明必须由本 crate 编译。
/// `harness` 依赖 `programs`，反向不可能。故这一族的声明住这里（一份，不拆 23 份：
/// "紧挨着身子"对身子不在本 crate 的那几台本来就不成立，不假装）。
#[path = "decl/harness.rs"]
pub mod harness;
#[path = "system/hub/program.rs"]
pub mod hub;
#[path = "system/operator/program.rs"]
pub mod operator;
#[path = "system/principal/program.rs"]
pub mod principal;
#[path = "root/program.rs"]
pub mod root;
#[path = "driver/router/program.rs"]
pub mod router;
#[path = "driver/rtc/program.rs"]
pub mod rtc;
#[path = "system/program.rs"]
pub mod system;
#[path = "driver/uart/program.rs"]
pub mod uart;

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
    /// 被指着的那一台**没有凭据**（`demand.setup` 空 ⇒ 它交不出"我答得动"）。
    NoEvidence(&'static str),
    /// 取不出可排的台 ⇒ 环（名字 = 卡住的那一个）。
    Cycle(&'static str),
}

/// **按 `deps` 把这一张单排成次序**（拓扑，原地重排）：每条边都在前面，`after_scene` 的排最后。
/// 同一批按**名字**排（与声明次序无关，可复现）。图上那三种说不通当场挑出来（[`DepsFail`]）。
///
/// **两个读者共用这一份**（照实记）：宿主那一侧打包时校验（`crates/image`，报得出名字），
/// 引导期那一趟排次序（`system::assemble`）。故它**不许分配**——本文件是宿主安全的
/// （只许引 `env`，`crates/image` 用 `#[path]` 文本包含它），只用切片与定长栈。
///
/// **它不解释任何一台的字段**：只读 [`Relation::deps`] / [`Relation::after_scene`] 两格。
pub fn order_scene(list: &mut [&'static Program]) -> Result<(), DepsFail> {
    // 一、每条边都要落得下：指得到本单里的台，且那一台说得出"我答得动"。
    let mut i = 0;
    while i < list.len() {
        if let Some(deps) = list[i].relation.deps {
            let mut d = 0;
            while d < deps.len() {
                let name = deps[d];
                match find(list, name) {
                    None => return Err(DepsFail::Unknown(name)),
                    Some(target) if target.demand.setup.is_empty() => {
                        return Err(DepsFail::NoEvidence(name))
                    }
                    Some(_) => {}
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
            if !list[i].relation.after_scene && ready(list, i, placed) {
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
    // 三、收尾：剩下的必须全是 `after_scene` 的（不是 ⇒ 环）；它们同批按名字。
    let mut i = placed;
    while i < n {
        if !list[i].relation.after_scene {
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

/// 这一台的**边都排好了吗**（`list[..placed]` 里找得到每一条边指着的那一台）。
fn ready(list: &[&'static Program], i: usize, placed: usize) -> bool {
    let Some(deps) = list[i].relation.deps else {
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
