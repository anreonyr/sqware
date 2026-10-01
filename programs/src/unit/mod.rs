//! program — **一台程序是什么**：它的全部装配声明，都写在它自己那份 `program.rs` 里。

use env::ProgramKind;

/// 装配失败的编号——`env::Reason` 的别名。
/// 装配编号只是"域自己的小整数"那一族（见 [`env::exit`] 的头注），不另立类型；名字留着是因为
/// 这张表通篇讲的是"哪一台、死在第几步"。
pub type Died = env::Reason;

/// **一台程序**：它的身份、它在装配图里的边、它起手要什么——**三块分开**。
/// ```text
///   Identity   它是谁（清单名 / 单元类型 / 特权空间 / 进哪几张景 / 是不是引导镜像）
///   Relation   它跟谁有边（依赖 / 存在信号 / 身份）
///   Demand     它起手要什么（死在第几步 / 那几手 setup）
/// ```
/// **它没有"代码在哪儿"那一格，也没有"从哪本账来"那一格**：那一段字节由**这一景那本账**给
/// （[`crate::system::source`] 取字节那一面——内核按 ELF 段现读，镜像一个字节都不被拷走）。
#[derive(Clone, Copy)]
pub struct UnitFile {
    /// **身份**：它是谁（宿主那侧只读这一块）。
    pub identity: Identity,
    /// **装配关系**：编排域把它接进来时那几条边。
    pub relation: Relation,
    /// **需求**：它起手要什么、身子从哪来、死在装配哪一步。
    pub demand: Demand,
}

impl UnitFile {
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
    pub fn listed(&self) -> bool {
        self.relation.after.is_some()
    }
}

/// **单元类型**——systemd 那份模型里的 `.service` / `.target`（后缀定了型）。
/// **两个变体各有生产者与读者**（它不是枚举摆设）：
/// | 变体 | 是什么 | 生产者 | 读者 |
/// |---|---|---|---|
/// | [`Kind::Service`] | 要起的服务（有身子、进镜像） | 其余每一份声明（吃 [`Identity::DEFAULT`]） | 宿主打包（`crates/image` 按它滤）／装配那一趟 |
/// | [`Kind::Target`] | **只把几条边聚在一起的目标**（没有身子、不进任何镜像） | [`SCENE_UNIT`] 一处 | [`is_target`]（`order_scene` 与装配那一趟据它认那条"等这一趟走完"的边） |
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// 要起的服务：有身子（进镜像），由编排域按 `after` 起。
    Service,
    /// **目标**：没有身子、不进任何镜像，只把几条边聚在一起（"这一趟装配走完"就是它的内容）。
    Target,
}

/// **身份**：这一台是谁——清单名、**单元类型**（[`Kind`]）、装成哪种**空间**、
/// 进哪几张景、是不是引导镜像。
/// **它是谁与它怎么被接进来是两件事**：`crates/image` 那台宿主只读这一块（且只走
/// [`UnitFile::name`] 那**四**条窄面：`name` / `space` / `wanted_by` / `entry`），装配关系与需求
/// 一概与打包无关。
#[derive(Clone, Copy)]
pub struct Identity {
    pub name: &'static str,
    /// **单元类型**（[`Kind`]）：`Service` 是要起的服务，`Target` 只把几条边聚在一起——
    /// **没有身子、不进任何镜像**（今天只有一个：本文件末尾那个 [`SCENE_UNIT`]）。
    pub kind: Kind,
    /// 装成哪种**空间**（S / U）。
    pub space: ProgramKind,
    /// **`WantedBy=`**（systemd 同名那一格）：**哪几张景要我**（景名即 target 名——`SCENE` 那一台
    /// 就是 `.target`，见 [`Kind::Target`]）——**次序即装载次序**。
    pub wanted_by: &'static [&'static str],
    /// **它是哪几张景的领头那一台**（**多数为空：全仓只有 7 处写它**）。一个景存在 ⇔ 它有一条
    pub entry: &'static [&'static str],
}

/// **谁结束它**——这一台的寿命由谁定。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ending {
    /// 我收它：只有被收才走（两个域 ＋ 七台服务）。
    Resident,
    Transient,
    /// 有人叫它走：终止词从外面来（控制台那一台）。
    Told,
}

/// **这一趟装配本身**（名单里那个目标单元的名字）：写法是 `after: Some(&[…, SCENE])` =
/// "**等这一趟装配走完**再起我"。今天只有 `probe-control` 一家写它：它问的
/// `/svc/sys/control/state` 由装配者在**整表起完之后**才铸、且要等监督那一趟开始才被**服务**
/// ——而"整表起完"不是一个台，图里本来没有它的落点。
/// **它是 [`Kind::Target`] 那个单元的名字**（[`SCENE_UNIT`]）：两边写的是同一个词，一处给
/// （静态那份声明的 `identity.name` 就是它）。
/// **到点是什么意思**（唯一一处）：[`order_scene`] 把写它的台排到**最后**（同批按名字）；编排域
/// 起完它们之后立刻挂上 control 那一面、进监督那一趟（`system/main.rs` 的相四）。**装配那一趟里
/// 不等它**——[`Assembly::assemble`](crate::system::Assembly::assemble) 逐条边等"那一台答得动"时
/// 跳过它：等一个"这一趟"没有可等的对象，**排到最后就是它的全部保证**。
pub const SCENE: &str = "scene";

/// **这一趟装配本身**——名单里的那个[目标单元](Kind::Target)：**没有身子、不进任何镜像**（宿主
/// 那一侧按 `wanted_by` 与 `kind` 两格把它滤掉），它对这张单的贡献只有一件事：**给"这一趟走完"
/// 一个落点**（[`SCENE`] 那条边指着它）。
pub static SCENE_UNIT: UnitFile = UnitFile {
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
/// 图里两种点：**要起的服务**（在单上、有身子）与**目标**（不在单上：它只把几条边聚在一起，
/// "这一趟装配走完"就是它的内容）。两个读者判的是同一句话：[`order_scene`] 据它把这一台排到
/// 最后（那一格要到那时才到点），装配那一趟据它跳过那一条边
/// （[`Assembly::assemble`](crate::system::Assembly::assemble)：等一个"这一趟"没有可等的对象）。
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
/// **这一块只有装配者读**：依赖 / 存在信号 / 结束方式 / 身份——都是
/// "这一台与那一台之间有一条什么边"，与它自己是谁（[`Identity`]）、起手要什么（[`Demand`]）分开。
#[derive(Clone, Copy)]
pub struct Relation {
    /// **`After=`**（systemd 同名那一格）：**这一台要等哪几位答得动**——起手那一趟的次序由它算
    /// 出来（**不再手排位次**）。
    /// 一条边 = "我起手的一个动作要求它已经**答得动**"；成立的凭据是那一台自己交回的那枚孔
    /// （[`Setup::Ready`] / [`Setup::Machine`] 的第二条）。`None` = **不在这一趟装配单上**
    pub after: Option<&'static [&'static str]>,
    /// **`Restart=`**（systemd 同名那一格）：**这一台的寿命由谁定**——`None` = 没声明；
    /// **由编排域起的台必须写**（`Control::enlist` 当场拒）。取值是 [`Ending`] 那一型
    pub restart: Option<Ending>,
}

/// **需求**：实例化它要多做的那几手、它死在装配哪一步的号。
/// **这一块只有装配者读**，两格答的是同一句话的两面："把它弄起来要动用什么"。
#[derive(Clone, Copy)]
pub struct Demand {
    pub supply: &'static [Setup],
}

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
    pub const DEFAULT: Relation = Relation {
        after: None,
        restart: None,
    };
}

impl Demand {
    pub const DEFAULT: Demand = Demand { supply: &[] };
}

/// 实例化一台要多做的一手。
/// **它不负责 start**：两种都在装配那一趟里按次序落到具体那几手上（见
/// [`crate::system::control`] 的 `connect_all` / `enroll`）。
pub const READY: &str = "ready";

#[derive(Clone, Copy)]
pub enum Setup {
    /// **答得动了**：这一台交回一枚刻 `READY` 的孔 ⇒ "**我这一面已经在树上、答得动**"。
    Ready,
    /// **整机物料**：这一台起手要**这台机器的全部可领之物**（设备树本体 / 门铃 / 每一台设备
    /// 那一段区）。
    /// 与 [`Setup::Ready`] 是**同一手 ＋ 一件事**：放行前照样 `connect`（它交回那一枚照样是
    /// "我起来了"），放行之后装配者多走一趟——**照 [`crate::system::machine::Machine::devices`]
    /// 枚举全机**、逐段向引导域领、再把那一段记录从这条通道推给它。
    Machine {
        /// 收物料那条通道的名字。
        load: &'static str,
        /// **"我起完了"那条通道**的名字（收方在起手末尾铸一枚刻它的孔）。
        ready: &'static str,
    },
}

impl Setup {
    pub const fn channel(&self) -> &'static str {
        match self {
            Setup::Ready => READY,
            Setup::Machine { load, .. } => load,
        }
    }

    /// **还有第二条吗**——`Machine` 那一格多一条（"**我起完了**"那条，见它自己的注）；
    pub const fn ready(&self) -> Option<&'static str> {
        match self {
            Setup::Ready => None,
            Setup::Machine { ready, .. } => Some(ready),
        }
    }

    pub const fn machine(&self) -> bool {
        matches!(self, Setup::Machine { .. })
    }
}

mod catalog;
pub use catalog::*;

mod order;
pub use order::*;

/// **推出来的那一格**：只有一条 `Ready`（[`UnitFile::supply`] 用它）。
static READY_ONLY: &[Setup] = &[Setup::Ready];

impl UnitFile {
    /// **这一台要交的凭据（有效值）**：声明里写的；**空了就按推导答**——被某一台的 `after`
    pub fn supply(&self) -> &'static [Setup] {
        if !self.demand.supply.is_empty() {
            self.demand.supply
        } else if crate::unit::order::needs_evidence(self.name()) {
            READY_ONLY
        } else {
            &[]
        }
    }
}
