//! harness — **测具那 25 台**的装配声明（一份）。
//! # 它为什么住这一份（而不是一台一份 `program.rs`）
//! 这 25 台的身子住 `programs/src/harness/`（探针 / 试客 / 压测台），其中 **15 台由编排域起**
//! （`guest` / `passer` / `lodger` / `sleeper` / `subject` / `member` 与七台 `probe-*`）——
//! 编排域要按 `order` / 存在信号（`Relation::presence`）/ `bind` / `died` 起它们，而
//! "哪几台进哪张镜像"那张表在宿主侧的 `crates/image` 里也要看得见 ⇒ 声明必须能在**只引 `env`**
//! 的前提下被编译（见 `unit/mod.rs` 那条纪律）。
//! 那 25 份身子是**独立的 bin 目标**（各自带 `#![no_std]` / `#![no_main]` 这类 crate 级属性），
//! **当不了模块**被 `#[path]` 拉进这棵树；要"一台一份"就得给每台开一个目录放 `program.rs`
//! ——今天不这么做，25 台的声明就住这一份。
//! 其余 10 台（`churn` / `rig` / `busy` / `park` / `hang` / `load` / `beat` / `again` /
//! `waiter` / `group`）**不由编排域起**（`order: None`）——它们是景的引导镜像或台主的受害者，
//! 声明仍在这里，好让 image 那侧一张表看全。

use crate::unit::{Demand, Died, Ending, Identity, Relation, SCENE,  UnitFile};
use env::ProgramKind;

pub const E_GUEST: Died = 7;
pub const E_PASSER: Died = 8;
pub const E_LODGER: Died = 11;
pub const E_SLEEPER: Died = 13;
pub const E_SUBJECT: Died = 15;
pub const E_MEMBER: Died = 17;
pub const E_PROBE: Died = 18;
pub const E_PROBE_OWNER: Died = 19;
pub const E_PROBE_LEASE: Died = 20;
pub const E_PROBE_RULE: Died = 21;
pub const E_PROBE_OTHER: Died = 22;
pub const E_PROBE_BOUND: Died = 23;
pub const E_PROBE_CONTROL: Died = 25;
pub const E_PROBE_OPERATOR_GATE: Died = 26;
pub const E_PROBE_OPERATOR_LAND: Died = 27;

pub static GUEST: UnitFile = UnitFile {
    identity: Identity {
        name: "guest",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "router"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};

/// 过客：起来、挂一个名字、**直接死**（不说再见）。
pub static PASSER: UnitFile = UnitFile {
    identity: Identity {
        name: "passer",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&[]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};

/// 房客：占一条线、**直接死**——线路由者那本账的探活读数。
pub static LODGER: UnitFile = UnitFile {
    identity: Identity {
        name: "lodger",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "hub", "router"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};

/// 客人：`/svc/drv/rtc` 那面服务的第一位用家。
pub static SLEEPER: UnitFile = UnitFile {
    identity: Identity {
        name: "sleeper",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "rtc"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};

/// 主体：身份服务的第一位真客人。
pub static SUBJECT: UnitFile = UnitFile {
    identity: Identity {
        name: "subject",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "principal"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};

/// 盟友：结盟服务的第一位真客人。
pub static MEMBER: UnitFile = UnitFile {
    identity: Identity {
        name: "member",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "coalition", "principal"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};

/// 一位**没有身份**的任务去撞树的门（`bind: false`）——"没绑身份 ⇒ 拒绝"的反例。
pub static PROBE_DENIED: UnitFile = UnitFile {
    identity: Identity {
        name: "probe-denied",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};

/// **有身份**、但那一格归别人（声明过归属）⇒ 也拒。
pub static PROBE_OWNER: UnitFile = UnitFile {
    identity: Identity {
        name: "probe-owner",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "uart"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};

/// 有身份的一台把 `Permit::Trunk` / `Bough` / `Among` 三条许可落下去（先正证、再负证）。
pub static PROBE_RULE: UnitFile = UnitFile {
    identity: Identity {
        name: "probe-rule",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "principal"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand {
        // **答得动**：`probe-rule-other` 读的那几格由本台落——落完才交这一枚（与三台驱动同一手）。
        ..Demand::DEFAULT
    },
};

/// 有身份地去用别人立了规矩的那两格 ⇒ 都该拒（第二道门的反例）。
pub static PROBE_RULE_OTHER: UnitFile = UnitFile {
    identity: Identity {
        name: "probe-rule-other",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "probe-rule"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};

/// 会死的持有者：落一块**声明归自己**的门牌然后直接死，好让下一台接手。
pub static PROBE_LEASE: UnitFile = UnitFile {
    identity: Identity {
        name: "probe-lease",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};

/// **上界的证客**：推一页 + 1、再推一枚不合族的帧到**持树者那几面**上。
pub static PROBE_BOUND: UnitFile = UnitFile {
    identity: Identity {
        name: "probe-bound",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};

/// **控制面的真客人**：从树上找 **`/svc/sys/control/state`**（问面），问一句 control 的话；
/// 另取那三面各期望被拒（带规矩），并拿问面发写、期望判面拒。
/// task-4 那条挂载路挂出过一块**查得到、取不回**的门牌（铸入口的是一枚一次性边沿线程，
/// 它一收尾，持树者表里那枚副本就被内核的派生链级联摘掉）。这一台量的正是那件事的反面：
/// **在另一个域里**照 principal / coalition 逐字同形的路找上门、把门牌取回来、问一句话。
/// 判据两条（`harness/src/probe/probe_control.rs`）：表外那个名字答 `Unknown`、本台自己答得出一个
/// 生命阶段——`Bad`（这一趟没走到对面）在两条里都是红。
/// **它排在最后**（`after` 里那条 [`SCENE`] 边）：那一面是在**整表起完**之后才挂上树的
/// （`Assembly::mount_control`，由 `system/main.rs` 的相四叫）——那**不是一个台**，图里本来
/// 没有它的落点，故写成"等装配那一趟走完"那条边（名字 [`SCENE`]，次序由
pub static PROBE_CONTROL: UnitFile = UnitFile {
    identity: Identity {
        name: "probe-control",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", SCENE]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};

/// **操作面的正证客人（全操作面那一半）**：拿控制面会话把 `/svc/sys/operator` 与它底下那几格看
/// 一眼、取回 `/svc/sys/operator/land` 那一枚入口、再把试验场（**根**底下两格归属不同的砖）铺好
/// 给下一位客人。
/// **它读的不是"整表起完"那一趟**：那七格挂在**持树者一就位**那一趟（`Assembly::mount_grants`），
/// 故它一上来就看得见；**那七段名字的读数归树自己**
/// ——本台只数一次格子（见 [`PROBE_OPERATOR_GATE`] 那一族的文件头与
pub static PROBE_OPERATOR_GATE: UnitFile = UnitFile {
    identity: Identity {
        name: "probe-operator-gate",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};

/// **操作面的正证客人（只有 `land` 一位那一半）**：会话开在 `granted_berth(Land)` 上，
/// 于是 `seek` / `part` / `find` / `trim` 全答 `Denied`，而 `land` 在**无主**那一格上通、
/// 在**别人有主**那一格上拒——后者证的是"面判与归属那一条轴**正交**"。
pub static PROBE_OPERATOR_LAND: UnitFile = UnitFile {
    identity: Identity {
        name: "probe-operator-land",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};

pub static CHURN: UnitFile = UnitFile {
    identity: Identity {
        name: "churn",
        wanted_by: &["again"],
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};

pub static RIG: UnitFile = UnitFile {
    identity: Identity {
        name: "rig",
        space: ProgramKind::Supervisor,
        wanted_by: &["rig"],
        entry: &["rig"],
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};

pub static BUSY: UnitFile = UnitFile {
    identity: Identity {
        name: "busy",
        wanted_by: &["load"],
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};

pub static PARK: UnitFile = UnitFile {
    identity: Identity {
        name: "park",
        wanted_by: &["load"],
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};

pub static HANG: UnitFile = UnitFile {
    identity: Identity {
        name: "hang",
        wanted_by: &["rig"],
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};

pub static LOAD: UnitFile = UnitFile {
    identity: Identity {
        name: "load",
        space: ProgramKind::Supervisor,
        wanted_by: &["load"],
        entry: &["load"],
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};

pub static BEAT: UnitFile = UnitFile {
    identity: Identity {
        name: "beat",
        space: ProgramKind::Supervisor,
        wanted_by: &["beat"],
        entry: &["beat"],
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};

pub static AGAIN: UnitFile = UnitFile {
    identity: Identity {
        name: "again",
        space: ProgramKind::Supervisor,
        wanted_by: &["again"],
        entry: &["again"],
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};

pub static WAITER: UnitFile = UnitFile {
    identity: Identity {
        name: "waiter",
        wanted_by: &["group"],
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};

pub static GROUP: UnitFile = UnitFile {
    identity: Identity {
        name: "group",
        space: ProgramKind::Supervisor,
        wanted_by: &["group"],
        entry: &["group"],
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};
