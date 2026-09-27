//! decl::harness — **测具那 22 台**的装配声明。
//!
//! # 它为什么住本 crate（而不是隔壁 `harness`）
//!
//! 这 22 台的身子住 `harness`，但其中 **12 台由编排域起**（`guest` / `passer` / `lodger` /
//! `sleeper` / `subject` / `member` 与六台 `probe-*`）——编排域要按 `order` / 存在信号
//! （`Relation::presence`）/ `bind` / `died` 起它们，故这几格必须由 `programs` 编译得出来。而 `harness` 依赖
//! `programs`，反向建依赖是环 ⇒ **声明只能住这一侧**。
//!
//! **为什么不拆成 22 份**：「一台一份 `program.rs`」的判据是"声明紧挨着它的身子"；这几台的
//! 身子**不在本 crate**，那句话对它们本来就不成立，不假装。可 grep 的那条规矩因此是：
//! **声明跟着"起它的那一侧"走**——产品程序由编排域起（身子也在本 crate）⇒ 住各自目录；
//! 测具由编排域起（身子在隔壁）⇒ 住本文件。
//!
//! 其余 10 台（`churn` / `rig` / `busy` / `park` / `hang` / `load` / `beat` / `again` /
//! `waiter` / `group`）**不由编排域起**（`order: None`）——它们是景的引导镜像或台主的受害者，
//! 声明仍在这里，好让 image 那侧一张表看全。

use crate::program::{Demand, Died, Identity, Origin, Program, Relation, Setup, Spot};
use env::ProgramKind;
use env::supply::{Kind, Need, class_block};
use env::{Access, Policy};

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

/// 房客要的那一枚：**一条没人要的线**（`virtio,mmio`），领上就死。
pub const LODGER_WANTS: &[Need] = &[Need::class(
    class_block("virtio,mmio"),
    Kind::Pole,
    Access::FETCH,
    Policy::ONLY,
)];

// ── 常客（进验收镜像当客人跑，量的是服务）────────────────────────────────

pub static GUEST: Program = Program {
    identity: Identity {
        name: "guest",
        kind: ProgramKind::User,
        spot: Spot::Guest,
        scenes: &["root"],
        entry: &[],
    },
    relation: Relation {
        order: Some(6),
        presence: true,
        operator: true,
        bind: true,
        holds_tree: false,
        eyes: None,
    },
    demand: Demand {
        origin: Origin::Initrd,
        died: E_GUEST,
        setup: &[],
    },
};

/// 过客：起来、挂一个名字、**直接死**（不说再见）。
pub static PASSER: Program = Program {
    identity: Identity {
        name: "passer",
        kind: ProgramKind::User,
        spot: Spot::Guest,
        scenes: &["root"],
        entry: &[],
    },
    relation: Relation {
        order: Some(7),
        presence: true,
        operator: false,
        bind: true,
        holds_tree: false,
        eyes: None,
    },
    demand: Demand {
        origin: Origin::Initrd,
        died: E_PASSER,
        setup: &[],
    },
};

/// 房客：占一条线、**直接死**——线路由者那本账的探活读数。
pub static LODGER: Program = Program {
    identity: Identity {
        name: "lodger",
        kind: ProgramKind::User,
        spot: Spot::Guest,
        scenes: &["root"],
        entry: &[],
    },
    relation: Relation {
        order: Some(8),
        presence: false,
        operator: true,
        bind: true,
        holds_tree: false,
        eyes: None,
    },
    demand: Demand {
        origin: Origin::Initrd,
        died: E_LODGER,
        setup: &[Setup::Need(LODGER_WANTS[0]), Setup::Channel("records")],
    },
};

/// 客人：`/device/rtc` 那面服务的第一位用家。
pub static SLEEPER: Program = Program {
    identity: Identity {
        name: "sleeper",
        kind: ProgramKind::User,
        spot: Spot::Guest,
        scenes: &["root"],
        entry: &[],
    },
    relation: Relation {
        order: Some(9),
        presence: true,
        operator: true,
        bind: true,
        holds_tree: false,
        eyes: None,
    },
    demand: Demand {
        origin: Origin::Initrd,
        died: E_SLEEPER,
        setup: &[],
    },
};

/// 主体：身份服务的第一位真客人。
pub static SUBJECT: Program = Program {
    identity: Identity {
        name: "subject",
        kind: ProgramKind::User,
        spot: Spot::Guest,
        scenes: &["root"],
        entry: &[],
    },
    relation: Relation {
        order: Some(10),
        presence: false,
        operator: true,
        bind: true,
        holds_tree: false,
        eyes: None,
    },
    demand: Demand {
        origin: Origin::Initrd,
        died: E_SUBJECT,
        setup: &[],
    },
};

/// 盟友：结盟服务的第一位真客人。
pub static MEMBER: Program = Program {
    identity: Identity {
        name: "member",
        kind: ProgramKind::User,
        spot: Spot::Guest,
        scenes: &["root"],
        entry: &[],
    },
    relation: Relation {
        order: Some(11),
        presence: false,
        operator: true,
        bind: true,
        holds_tree: false,
        eyes: None,
    },
    demand: Demand {
        origin: Origin::Initrd,
        died: E_MEMBER,
        setup: &[],
    },
};

// ── 探针（只读数；负证那一族）────────────────────────────────────────────

/// 一位**没有身份**的任务去撞树的门（`bind: false`）——"没绑身份 ⇒ 拒绝"的反例。
pub static PROBE_DENIED: Program = Program {
    identity: Identity {
        name: "probe-denied",
        kind: ProgramKind::User,
        spot: Spot::Probe,
        scenes: &["root"],
        entry: &[],
    },
    relation: Relation {
        order: Some(12),
        presence: false,
        operator: true,
        bind: false,
        holds_tree: false,
        eyes: None,
    },
    demand: Demand {
        origin: Origin::Initrd,
        died: E_PROBE,
        setup: &[],
    },
};

/// **有身份**、但那一格归别人（`Rule::Owner`）⇒ 也拒。
pub static PROBE_OWNER: Program = Program {
    identity: Identity {
        name: "probe-owner",
        kind: ProgramKind::User,
        spot: Spot::Probe,
        scenes: &["root"],
        entry: &[],
    },
    relation: Relation {
        order: Some(14),
        presence: false,
        operator: true,
        bind: true,
        holds_tree: false,
        eyes: None,
    },
    demand: Demand {
        origin: Origin::Initrd,
        died: E_PROBE_OWNER,
        setup: &[],
    },
};

/// 有身份的一台把 `Is` / `Under` / `In` 三条规矩落下去（先正证、再负证）。
pub static PROBE_RULE: Program = Program {
    identity: Identity {
        name: "probe-rule",
        kind: ProgramKind::User,
        spot: Spot::Probe,
        scenes: &["root"],
        entry: &[],
    },
    relation: Relation {
        order: Some(15),
        presence: false,
        operator: true,
        bind: true,
        holds_tree: false,
        eyes: None,
    },
    demand: Demand {
        origin: Origin::Initrd,
        died: E_PROBE_RULE,
        setup: &[],
    },
};

/// 有身份地去用别人立了规矩的那两格 ⇒ 都该拒（第二道门的反例）。
pub static PROBE_RULE_OTHER: Program = Program {
    identity: Identity {
        name: "probe-rule-other",
        kind: ProgramKind::User,
        spot: Spot::Probe,
        scenes: &["root"],
        entry: &[],
    },
    relation: Relation {
        order: Some(16),
        presence: false,
        operator: true,
        bind: true,
        holds_tree: false,
        eyes: None,
    },
    demand: Demand {
        origin: Origin::Initrd,
        died: E_PROBE_OTHER,
        setup: &[],
    },
};

/// 会死的持有者：落一块**声明归自己**的门牌然后直接死，好让下一台接手。
pub static PROBE_LEASE: Program = Program {
    identity: Identity {
        name: "probe-lease",
        kind: ProgramKind::User,
        spot: Spot::Probe,
        scenes: &["root"],
        entry: &[],
    },
    relation: Relation {
        order: Some(13),
        presence: false,
        operator: true,
        bind: true,
        holds_tree: false,
        eyes: None,
    },
    demand: Demand {
        origin: Origin::Initrd,
        died: E_PROBE_LEASE,
        setup: &[],
    },
};

/// **上界的证客**：推一页 + 1、再推一枚不合族的帧到**两道门**（树与板）上。
/// 两道门各一条腿，故这一台要两条路（`operator` ＋ `board`）。
pub static PROBE_BOUND: Program = Program {
    identity: Identity {
        name: "probe-bound",
        kind: ProgramKind::User,
        spot: Spot::Probe,
        scenes: &["root"],
        entry: &[],
    },
    relation: Relation {
        order: Some(17),
        presence: true,
        operator: true,
        bind: true,
        holds_tree: false,
        eyes: None,
    },
    demand: Demand {
        origin: Origin::Initrd,
        died: E_PROBE_BOUND,
        setup: &[],
    },
};

// ── 压测台与它们的受害者（整台替换引导镜像）────────────────────────────

pub static CHURN: Program = Program {
    identity: Identity {
        name: "churn",
        kind: ProgramKind::User,
        spot: Spot::Rig,
        scenes: &["again"],
        entry: &[],
    },
    relation: Relation {
        order: None,
        presence: false,
        operator: false,
        bind: false,
        holds_tree: false,
        eyes: None,
    },
    demand: Demand {
        origin: Origin::Initrd,
        died: env::EXIT_OK,
        setup: &[],
    },
};

pub static RIG: Program = Program {
    identity: Identity {
        name: "rig",
        kind: ProgramKind::Supervisor,
        spot: Spot::Rig,
        scenes: &["rig"],
        entry: &["rig"],
    },
    relation: Relation {
        order: None,
        presence: false,
        operator: false,
        bind: false,
        holds_tree: false,
        eyes: None,
    },
    demand: Demand {
        origin: Origin::Initrd,
        died: env::EXIT_OK,
        setup: &[],
    },
};

pub static BUSY: Program = Program {
    identity: Identity {
        name: "busy",
        kind: ProgramKind::User,
        spot: Spot::Rig,
        scenes: &["load"],
        entry: &[],
    },
    relation: Relation {
        order: None,
        presence: false,
        operator: false,
        bind: false,
        holds_tree: false,
        eyes: None,
    },
    demand: Demand {
        origin: Origin::Initrd,
        died: env::EXIT_OK,
        setup: &[],
    },
};

pub static PARK: Program = Program {
    identity: Identity {
        name: "park",
        kind: ProgramKind::User,
        spot: Spot::Rig,
        scenes: &["load"],
        entry: &[],
    },
    relation: Relation {
        order: None,
        presence: false,
        operator: false,
        bind: false,
        holds_tree: false,
        eyes: None,
    },
    demand: Demand {
        origin: Origin::Initrd,
        died: env::EXIT_OK,
        setup: &[],
    },
};

pub static HANG: Program = Program {
    identity: Identity {
        name: "hang",
        kind: ProgramKind::User,
        spot: Spot::Rig,
        scenes: &["rig"],
        entry: &[],
    },
    relation: Relation {
        order: None,
        presence: false,
        operator: false,
        bind: false,
        holds_tree: false,
        eyes: None,
    },
    demand: Demand {
        origin: Origin::Initrd,
        died: env::EXIT_OK,
        setup: &[],
    },
};

pub static LOAD: Program = Program {
    identity: Identity {
        name: "load",
        kind: ProgramKind::Supervisor,
        spot: Spot::Rig,
        scenes: &["load"],
        entry: &["load"],
    },
    relation: Relation {
        order: None,
        presence: false,
        operator: false,
        bind: false,
        holds_tree: false,
        eyes: None,
    },
    demand: Demand {
        origin: Origin::Initrd,
        died: env::EXIT_OK,
        setup: &[],
    },
};

pub static BEAT: Program = Program {
    identity: Identity {
        name: "beat",
        kind: ProgramKind::Supervisor,
        spot: Spot::Rig,
        scenes: &["beat"],
        entry: &["beat"],
    },
    relation: Relation {
        order: None,
        presence: false,
        operator: false,
        bind: false,
        holds_tree: false,
        eyes: None,
    },
    demand: Demand {
        origin: Origin::Initrd,
        died: env::EXIT_OK,
        setup: &[],
    },
};

pub static AGAIN: Program = Program {
    identity: Identity {
        name: "again",
        kind: ProgramKind::Supervisor,
        spot: Spot::Rig,
        scenes: &["again"],
        entry: &["again"],
    },
    relation: Relation {
        order: None,
        presence: false,
        operator: false,
        bind: false,
        holds_tree: false,
        eyes: None,
    },
    demand: Demand {
        origin: Origin::Initrd,
        died: env::EXIT_OK,
        setup: &[],
    },
};

pub static WAITER: Program = Program {
    identity: Identity {
        name: "waiter",
        kind: ProgramKind::User,
        spot: Spot::Rig,
        scenes: &["group"],
        entry: &[],
    },
    relation: Relation {
        order: None,
        presence: false,
        operator: false,
        bind: false,
        holds_tree: false,
        eyes: None,
    },
    demand: Demand {
        origin: Origin::Initrd,
        died: env::EXIT_OK,
        setup: &[],
    },
};

pub static GROUP: Program = Program {
    identity: Identity {
        name: "group",
        kind: ProgramKind::Supervisor,
        spot: Spot::Rig,
        scenes: &["group"],
        entry: &["group"],
    },
    relation: Relation {
        order: None,
        presence: false,
        operator: false,
        bind: false,
        holds_tree: false,
        eyes: None,
    },
    demand: Demand {
        origin: Origin::Initrd,
        died: env::EXIT_OK,
        setup: &[],
    },
};
