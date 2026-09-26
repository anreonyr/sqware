//! decl::harness — **测具那 22 台**的装配声明。
//!
//! # 它为什么住本 crate（而不是隔壁 `harness`）
//!
//! 这 22 台的身子住 `harness`，但其中 **12 台由编排域起**（`guest` / `passer` / `lodger` /
//! `sleeper` / `subject` / `member` 与六台 `probe-*`）——编排域要按 `order` / `board` /
//! `bind` / `died` 起它们，故这几格必须由 `programs` 编译得出来。而 `harness` 依赖
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

use crate::program::{Died, Program, Setup, Spot};
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
    name: "guest",
    kind: ProgramKind::User,
    spot: Spot::Guest,
    scenes: &["root"],
    entry: &[],
    order: Some(6),
    board: true,
    operator: true,
    bind: true,
    holds_tree: false,
    eyes: None,
    died: E_GUEST,
    setup: &[],
};

/// 过客：起来、挂一个名字、**直接死**（不说再见）。
pub static PASSER: Program = Program {
    name: "passer",
    kind: ProgramKind::User,
    spot: Spot::Guest,
    scenes: &["root"],
    entry: &[],
    order: Some(7),
    board: true,
    operator: false,
    bind: true,
    holds_tree: false,
    eyes: None,
    died: E_PASSER,
    setup: &[],
};

/// 房客：占一条线、**直接死**——线路由者那本账的探活读数。
pub static LODGER: Program = Program {
    name: "lodger",
    kind: ProgramKind::User,
    spot: Spot::Guest,
    scenes: &["root"],
    entry: &[],
    order: Some(8),
    board: false,
    operator: true,
    bind: true,
    holds_tree: false,
    eyes: None,
    died: E_LODGER,
    setup: &[Setup::Need(LODGER_WANTS[0]), Setup::Channel("records")],
};

/// 客人：`/device/rtc` 那面服务的第一位用家。
pub static SLEEPER: Program = Program {
    name: "sleeper",
    kind: ProgramKind::User,
    spot: Spot::Guest,
    scenes: &["root"],
    entry: &[],
    order: Some(9),
    board: true,
    operator: true,
    bind: true,
    holds_tree: false,
    eyes: None,
    died: E_SLEEPER,
    setup: &[],
};

/// 主体：身份服务的第一位真客人。
pub static SUBJECT: Program = Program {
    name: "subject",
    kind: ProgramKind::User,
    spot: Spot::Guest,
    scenes: &["root"],
    entry: &[],
    order: Some(10),
    board: false,
    operator: true,
    bind: true,
    holds_tree: false,
    eyes: None,
    died: E_SUBJECT,
    setup: &[],
};

/// 盟友：结盟服务的第一位真客人。
pub static MEMBER: Program = Program {
    name: "member",
    kind: ProgramKind::User,
    spot: Spot::Guest,
    scenes: &["root"],
    entry: &[],
    order: Some(11),
    board: false,
    operator: true,
    bind: true,
    holds_tree: false,
    eyes: None,
    died: E_MEMBER,
    setup: &[],
};

// ── 探针（只读数；负证那一族）────────────────────────────────────────────

/// 一位**没有身份**的任务去撞树的门（`bind: false`）——"没绑身份 ⇒ 拒绝"的反例。
pub static PROBE_DENIED: Program = Program {
    name: "probe-denied",
    kind: ProgramKind::User,
    spot: Spot::Probe,
    scenes: &["root"],
    entry: &[],
    order: Some(12),
    board: false,
    operator: true,
    bind: false,
    holds_tree: false,
    eyes: None,
    died: E_PROBE,
    setup: &[],
};

/// **有身份**、但那一格归别人（`Rule::Owner`）⇒ 也拒。
pub static PROBE_OWNER: Program = Program {
    name: "probe-owner",
    kind: ProgramKind::User,
    spot: Spot::Probe,
    scenes: &["root"],
    entry: &[],
    order: Some(14),
    board: false,
    operator: true,
    bind: true,
    holds_tree: false,
    eyes: None,
    died: E_PROBE_OWNER,
    setup: &[],
};

/// 有身份的一台把 `Is` / `Under` / `In` 三条规矩落下去（先正证、再负证）。
pub static PROBE_RULE: Program = Program {
    name: "probe-rule",
    kind: ProgramKind::User,
    spot: Spot::Probe,
    scenes: &["root"],
    entry: &[],
    order: Some(15),
    board: false,
    operator: true,
    bind: true,
    holds_tree: false,
    eyes: None,
    died: E_PROBE_RULE,
    setup: &[],
};

/// 有身份地去用别人立了规矩的那两格 ⇒ 都该拒（第二道门的反例）。
pub static PROBE_RULE_OTHER: Program = Program {
    name: "probe-rule-other",
    kind: ProgramKind::User,
    spot: Spot::Probe,
    scenes: &["root"],
    entry: &[],
    order: Some(16),
    board: false,
    operator: true,
    bind: true,
    holds_tree: false,
    eyes: None,
    died: E_PROBE_OTHER,
    setup: &[],
};

/// 会死的持有者：落一块**声明归自己**的门牌然后直接死，好让下一台接手。
pub static PROBE_LEASE: Program = Program {
    name: "probe-lease",
    kind: ProgramKind::User,
    spot: Spot::Probe,
    scenes: &["root"],
    entry: &[],
    order: Some(13),
    board: false,
    operator: true,
    bind: true,
    holds_tree: false,
    eyes: None,
    died: E_PROBE_LEASE,
    setup: &[],
};

/// **上界的证客**：推一页 + 1、再推一枚不合族的帧到**两道门**（树与板）上。
/// 两道门各一条腿，故这一台要两条路（`operator` ＋ `board`）。
pub static PROBE_BOUND: Program = Program {
    name: "probe-bound",
    kind: ProgramKind::User,
    spot: Spot::Probe,
    scenes: &["root"],
    entry: &[],
    order: Some(17),
    board: true,
    operator: true,
    bind: true,
    holds_tree: false,
    eyes: None,
    died: E_PROBE_BOUND,
    setup: &[],
};

// ── 压测台与它们的受害者（整台替换引导镜像）────────────────────────────

pub static CHURN: Program = Program {
    name: "churn",
    kind: ProgramKind::User,
    spot: Spot::Rig,
    scenes: &["again"],
    entry: &[],
    order: None,
    board: false,
    operator: false,
    bind: false,
    holds_tree: false,
    eyes: None,
    died: env::EXIT_OK,
    setup: &[],
};

pub static RIG: Program = Program {
    name: "rig",
    kind: ProgramKind::Supervisor,
    spot: Spot::Rig,
    scenes: &["rig"],
    entry: &["rig"],
    order: None,
    board: false,
    operator: false,
    bind: false,
    holds_tree: false,
    eyes: None,
    died: env::EXIT_OK,
    setup: &[],
};

pub static BUSY: Program = Program {
    name: "busy",
    kind: ProgramKind::User,
    spot: Spot::Rig,
    scenes: &["load"],
    entry: &[],
    order: None,
    board: false,
    operator: false,
    bind: false,
    holds_tree: false,
    eyes: None,
    died: env::EXIT_OK,
    setup: &[],
};

pub static PARK: Program = Program {
    name: "park",
    kind: ProgramKind::User,
    spot: Spot::Rig,
    scenes: &["load"],
    entry: &[],
    order: None,
    board: false,
    operator: false,
    bind: false,
    holds_tree: false,
    eyes: None,
    died: env::EXIT_OK,
    setup: &[],
};

pub static HANG: Program = Program {
    name: "hang",
    kind: ProgramKind::User,
    spot: Spot::Rig,
    scenes: &["rig"],
    entry: &[],
    order: None,
    board: false,
    operator: false,
    bind: false,
    holds_tree: false,
    eyes: None,
    died: env::EXIT_OK,
    setup: &[],
};

pub static LOAD: Program = Program {
    name: "load",
    kind: ProgramKind::Supervisor,
    spot: Spot::Rig,
    scenes: &["load"],
    entry: &["load"],
    order: None,
    board: false,
    operator: false,
    bind: false,
    holds_tree: false,
    eyes: None,
    died: env::EXIT_OK,
    setup: &[],
};

pub static BEAT: Program = Program {
    name: "beat",
    kind: ProgramKind::Supervisor,
    spot: Spot::Rig,
    scenes: &["beat"],
    entry: &["beat"],
    order: None,
    board: false,
    operator: false,
    bind: false,
    holds_tree: false,
    eyes: None,
    died: env::EXIT_OK,
    setup: &[],
};

pub static AGAIN: Program = Program {
    name: "again",
    kind: ProgramKind::Supervisor,
    spot: Spot::Rig,
    scenes: &["again"],
    entry: &["again"],
    order: None,
    board: false,
    operator: false,
    bind: false,
    holds_tree: false,
    eyes: None,
    died: env::EXIT_OK,
    setup: &[],
};

pub static WAITER: Program = Program {
    name: "waiter",
    kind: ProgramKind::User,
    spot: Spot::Rig,
    scenes: &["group"],
    entry: &[],
    order: None,
    board: false,
    operator: false,
    bind: false,
    holds_tree: false,
    eyes: None,
    died: env::EXIT_OK,
    setup: &[],
};

pub static GROUP: Program = Program {
    name: "group",
    kind: ProgramKind::Supervisor,
    spot: Spot::Rig,
    scenes: &["group"],
    entry: &["group"],
    order: None,
    board: false,
    operator: false,
    bind: false,
    holds_tree: false,
    eyes: None,
    died: env::EXIT_OK,
    setup: &[],
};
