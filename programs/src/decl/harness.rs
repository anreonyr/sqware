//! decl::harness — **测具那 23 台**的装配声明。
//!
//! # 它为什么住本 crate（而不是隔壁 `harness`）
//!
//! 这 23 台的身子住 `harness`，但其中 **13 台由编排域起**（`guest` / `passer` / `lodger` /
//! `sleeper` / `subject` / `member` 与七台 `probe-*`）——编排域要按 `order` / 存在信号
//! （`Relation::presence`）/ `bind` / `died` 起它们，故这几格必须由 `programs` 编译得出来。而 `harness` 依赖
//! `programs`，反向建依赖是环 ⇒ **声明只能住这一侧**。
//!
//! **为什么不拆成 23 份**：「一台一份 `program.rs`」的判据是"声明紧挨着它的身子"；这几台的
//! 身子**不在本 crate**，那句话对它们本来就不成立，不假装。可 grep 的那条规矩因此是：
//! **声明跟着"起它的那一侧"走**——产品程序由编排域起（身子也在本 crate）⇒ 住各自目录；
//! 测具由编排域起（身子在隔壁）⇒ 住本文件。
//!
//! 其余 10 台（`churn` / `rig` / `busy` / `park` / `hang` / `load` / `beat` / `again` /
//! `waiter` / `group`）**不由编排域起**（`order: None`）——它们是景的引导镜像或台主的受害者，
//! 声明仍在这里，好让 image 那侧一张表看全。

use crate::unit::{Demand, Died, Ending, Identity, UnitFile, Relation, SCENE, Setup};
use env::ProgramKind;

// ── 号（装配期死法的号；各台的身子里一个数都不写）────────────────────────

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

// ── 常客（进验收镜像当客人跑，量的是服务）────────────────────────────────

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
    demand: Demand {
        died: E_GUEST,
        ..Demand::DEFAULT
    },
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
    demand: Demand {
        died: E_PASSER,
        ..Demand::DEFAULT
    },
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
    demand: Demand {
        died: E_LODGER,
        ..Demand::DEFAULT
    },
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
    demand: Demand {
        died: E_SLEEPER,
        ..Demand::DEFAULT
    },
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
    demand: Demand {
        died: E_SUBJECT,
        ..Demand::DEFAULT
    },
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
    demand: Demand {
        died: E_MEMBER,
        ..Demand::DEFAULT
    },
};

// ── 探针（只读数；负证那一族）────────────────────────────────────────────

/// 一位**没有身份**的任务去撞树的门（`bind: false`）——"没绑身份 ⇒ 拒绝"的反例。
///
/// **这一行 `bind: false` 从前是"没写"**（照实记见 `Relation::DEFAULT`）：全表唯一的反例靠
/// **省略**那一格表达，而它自己的注释里却写着 `bind: false` ⇒ 那一格翻成"正常那一档"之后，
/// 这里**明写**。
pub static PROBE_DENIED: UnitFile = UnitFile {
    identity: Identity {
        name: "probe-denied",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator"]),
        bind: false,
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand {
        died: E_PROBE,
        ..Demand::DEFAULT
    },
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
    demand: Demand {
        died: E_PROBE_OWNER,
        ..Demand::DEFAULT
    },
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
        died: E_PROBE_RULE,
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
    demand: Demand {
        died: E_PROBE_OTHER,
        ..Demand::DEFAULT
    },
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
    demand: Demand {
        died: E_PROBE_LEASE,
        ..Demand::DEFAULT
    },
};

/// **上界的证客**：推一页 + 1、再推一枚不合族的帧到**两道门**（树与板）上。
/// 两道门各一条腿，故这一台要两条路（`operator` ＋ `board`）。
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
    demand: Demand {
        died: E_PROBE_BOUND,
        ..Demand::DEFAULT
    },
};

/// **控制面的真客人**：从树上找 **`/svc/sys/control/state`**（问面），问一句 control 的话；
/// 另取那三面各期望被拒（带规矩），并拿问面发写、期望判面拒。
///
/// task-4 那条挂载路挂出过一块**查得到、取不回**的门牌（铸入口的是一枚一次性边沿线程，
/// 它一收尾，持树者表里那枚副本就被内核的派生链级联摘掉）。这一台量的正是那件事的反面：
/// **在另一个域里**照 principal / coalition 逐字同形的路找上门、把门牌取回来、问一句话。
/// 判据两条（`harness/src/probe_control.rs`）：表外那个名字答 `Unknown`、本台自己答得出一个
/// 生命阶段——`Bad`（这一趟没走到对面）在两条里都是红。
///
/// **它排在最后**（`after` 里那条 [`SCENE`] 边）：那一面是在**整表起完**之后才挂上树的
/// （`Assembly::mount_control`，由 `system/main.rs` 的相四叫）——那**不是一个台**，图里本来
/// 没有它的落点，故写成"等装配那一趟走完"那条边（名字 [`SCENE`]，次序由
/// `program::order_scene` 算；那一格从前是一格布尔 `after_scene`，照实记见 [`SCENE`] 的头注）。
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
    demand: Demand {
        died: E_PROBE_CONTROL,
        ..Demand::DEFAULT
    },
};

// ── 操作面那一族（`/svc/sys/operator/{part,land,…}`）─────────────────────

/// **操作面的正证客人（全操作面那一半）**：拿控制面会话把 `/svc/sys/operator` 与它底下那几格看
/// 一眼、取回 `/svc/sys/operator/land` 那一枚入口、再把试验场（**根**底下两格归属不同的砖）铺好
/// 给下一位客人。
///
/// **它读的不是"整表起完"那一趟**：那七格挂在**持树者一就位**那一趟（`Assembly::mount_grants`），
/// 故它一上来就看得见（实测七行 `grant mounted` 在 t<500 ms 就打完）；**那七段名字的读数归树自己**
/// ——本台只数一次格子（见 [`PROBE_OPERATOR_GATE`] 那一族的文件头与
/// `harness/src/probe_operator_gate.rs::count_under` 的照实记：逐个问名那一版是这条读数里
/// 唯一会掉的一环，走满额度被停机扳机扑杀 ⇒ 绿也没有、红也没有）。
///
/// 它排在 `some(3)`（**身份服务之后、三台驱动之前**）：它只跟树说话，不需要任何驱动；而"要读
/// 那几格"的客人**离停机扳机越近，窗口越窄**——扳机是那张单上最大 `order` 那一条
/// （见 `canonical/program.rs` 的照实记）。
///
/// 它铺的那两格落在**根**底下：下一位只持 `land` 一位 ⇒ 它**问不得** `list` / `seek` / `name`，
/// 故那两格必须落在**唯一不需要号的那一格**上（见 `harness/src/probe_operator_gate.rs` 文件头）。
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
    demand: Demand {
        died: E_PROBE_OPERATOR_GATE,
        ..Demand::DEFAULT
    },
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
    demand: Demand {
        died: E_PROBE_OPERATOR_LAND,
        ..Demand::DEFAULT
    },
};

// ── 压测台与它们的受害者（整台替换引导镜像）────────────────────────────

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
