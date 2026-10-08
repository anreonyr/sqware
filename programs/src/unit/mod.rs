//! 程序的静态装配声明：身份、关系、需求和发布权限。
//!
//! 声明跟着 program.rs：每个 bin 自有目录，声明就在那里。

use env::ProgramKind;

/// 装配失败号。**true = 死在这**——装配期那两步不能往里挂死。
pub type Died = env::Reason;

/// 宿主读取身份，Control 按关系和需求启动实例，并检查发布权限。
#[derive(Clone, Copy)]
pub struct UnitFile {
    /// 允许实例发布的入口；运行时身份由 Identity 绑定。
    pub publication: &'static [Publish],
    /// 身份：宿主只读这一块。
    pub identity: Identity,
    /// 装配关系：编排域读这一块。
    pub relation: Relation,
    /// 需求：装配者读这一块。
    pub demand: Demand,
}

impl UnitFile {
    /// 清单名。
    pub fn name(&self) -> &'static str {
        self.identity.name
    }

    /// 单元类型（`Service` / `Target`）。
    pub fn kind(&self) -> Kind {
        self.identity.kind
    }

    /// 跑在哪种特权空间（S / U）。**它不是单元类型**——单元类型见 [`Kind`]。
    pub fn space(&self) -> ProgramKind {
        self.identity.space
    }

    /// 进哪几张引导镜像（景名）。
    pub fn wanted_by(&self) -> &'static [&'static str] {
        self.identity.wanted_by
    }

    /// 它是哪几张景的领头那一台。
    pub fn entry(&self) -> &'static [&'static str] {
        self.identity.entry
    }

    pub fn listed(&self) -> bool {
        self.relation.after.is_some()
    }
}

/// 单元类型。`Service` 进镜像、由编排域起；`Target` 没有身子、不进镜像，只是把几条边聚在一起。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// 要起的服务：有身子（进镜像），由编排域按 `after` 起。
    Service,
    /// 目标：没有身子，不进镜像。
    Target,
}

/// 身份：清单名、单元类型、特权空间、进哪几张景、是不是引导镜像。
/// **身份 vs 接进来是两件事**：宿主只读身份，装配关系与需求与打包无关。
#[derive(Clone, Copy)]
pub struct Identity {
    pub name: &'static str,
    /// `Service` 是要起的服务，`Target` 只把几条边聚在一起。
    pub kind: Kind,
    /// 跑在哪种特权空间（S / U）。
    pub space: ProgramKind,
    /// `WantedBy=`——哪几张景要这台。次序即装载次序。
    pub wanted_by: &'static [&'static str],
    /// 领头那一台：每个景一条。一个景存在 ⇔ 它有一条领头台。
    pub entry: &'static [&'static str],
}

/// 谁结束它——这一台的寿命由谁定。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ending {
    /// 我收它：只有被收才走。
    Resident,
    Transient,
    /// 有人叫它走：终止词从外面来。
    Told,
}

/// 这一趟装配本身：名单里那个目标单元的名字。
///
/// `after: Some(&[…, SCENE])` 表示等待本景的其他单元就绪。
pub const SCENE: &str = "scene";

/// 名单里的目标单元：没有身子、不进任何镜像——它对这张单的贡献只有一件事：
/// 给"这一趟走完"一个落点（`SCENE` 那条边指着它）。
pub static SCENE_UNIT: UnitFile = UnitFile {
    publication: &[],
    identity: Identity {
        name: SCENE,
        kind: Kind::Target,
        wanted_by: &[],
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};

/// 这一条边指着的是不是"这一趟自己"——即那一台是不是目标单元 [`Kind::Target`]。
///
/// 图里两种点：**要起的服务**（在单上、有身子）与**目标**（不在单上：它只把几条边聚在一起）。
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

/// 装配关系——编排域把它接进来时那几条边。
#[derive(Clone, Copy)]
pub struct Relation {
    /// `After=`——这一台要等哪几位答得动。次序由它算出来。
    ///
    /// 一条边 = "我起手的一个动作要求它已经答得动"；成立的凭据是那一台自己交回的那枚孔。
    pub after: Option<&'static [&'static str]>,
    /// `Restart=`——这一台的寿命由谁定。`None` = 没声明；由编排域起的台必须写。
    pub restart: Option<Ending>,
}

/// 需求：实例化它要多做的那几手。
#[derive(Clone, Copy)]
pub struct Demand {
    pub supply: &'static [Setup],
}

impl Identity {
    pub const DEFAULT: Identity = Identity {
        name: "",
        kind: Kind::Service,
        space: ProgramKind::User,
        wanted_by: &["accept"],
        entry: &[],
    };
}

impl Relation {
    /// 什么都没声明的那一形：不在装配单上（`after: None`）、没说自己怎么结束（`restart: None`）。
    pub const DEFAULT: Relation = Relation {
        after: None,
        restart: None,
    };
}

impl Demand {
    pub const DEFAULT: Demand = Demand { supply: &[] };
}

/// 实例化一台要多做的一手——通道名。
pub const READY: &str = "ready";
/// 启动关系中的 ready 角色，不是发布权利。
pub const READY_MARK: env::Mark = env::Mark::of(READY);

#[derive(Clone, Copy)]
pub enum Setup {
    /// 答得动了：这一台交回一枚刻 `READY` 的孔。
    Ready,
    /// 整机物料：这一台起手要这台机器的全部可领之物。
    Machine {
        /// 收物料那条通道的名字。
        load: &'static str,
        /// "我起完了"那条通道的名字（收方在起手末尾铸一枚刻它的孔）。
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

    /// 还有第二条吗——`Machine` 多一格（"我起完了"）。
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
pub(crate) mod publication;
pub use publication::{Publish, PublishEntry, PublishScope};

/// 推导出来的凭据——只有一条 `Ready`（装配者按 `after` 反查时用）。
static READY_ONLY: &[Setup] = &[Setup::Ready];

impl UnitFile {
    /// 这一台要交的凭据：声明里写的；空了就按推导答——被某一台的 `after` 点过名就得交一条 [`Setup::Ready`]。
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
