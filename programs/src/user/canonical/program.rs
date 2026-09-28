//! canonical::program — **控制台那一台**（`prog-canonical`）的装配声明。
//!
//! **U 态**（最小特权）：只走树上那一族客手与 `env` 的调试面，够不着建域那道 S 态门。
//! 它进 `root` / `product` 两景（原先那台回显的位置），且是那两景**最后一条**（`order: 21`）
//! ——编排域等它退场才收场（它一退 ⇒ 引导域退 ⇒ 级联扑杀 ⇒ 停机）。
//!
//! **照实记（18 → 19 → 21：一路让给真客人）**：这一格是**停机那一格的触发源**
//! （`Watch::of` 的 `watch_last` ⇒ `list.last()`），故**它必须是那张单上的最大 `order`**。
//! 每多一位"要等服务起来才问得动"的真客人，就让位一次：`probe-control` 排在 20；而操作面那两位
//! （`probe-operator-gate` / `probe-operator-land`）排在**最前**（3 / 4，身份服务之后、驱动之前）
//! ——它们要在停机扳机响之前把那七格读完，离扳机越近窗口越窄。
//!
//! **本仓那个"收场口令喂得太早"的坑（照实记）**：`scripts/boot.nu` 单行那一档**一起机就喂**
//! （之后每 2 s 再喂），于是本台常常在整表还没起完时就吃掉那一行 ⇒ 编排域当场开始收场、把还在
//! 半路的域扑杀。症状不是"某台坏了"，而是**随机某几台没有读数**（实测：`probe-rule`、
//! `probe-control` 都中过；装配期的 `board:claim` / `uart start` 那几处也中过）。把喂入口令推迟
//! 到整表起完（`(sleep 20; echo exit) | nu scripts/boot.nu …`）之后，同一份镜像**每一台都有读数**。

use crate::program::{Demand, Died, Identity, Origin, Program, Relation, Spot};
use env::ProgramKind;

/// 它死在起手哪一步。
pub const E_CANONICAL: Died = 24;

pub static PROGRAM: Program = Program {
    identity: Identity {
        name: "canonical",
        kind: ProgramKind::User,
        spot: Spot::Console,
        scenes: &["root", "product"],
        entry: &[],
    },
    relation: Relation {
        order: Some(21),
        presence: true,
        operator: true,
        bind: true,
        holds_tree: false,
        eyes: None,
    },
    demand: Demand {
        origin: Origin::Initrd,
        died: E_CANONICAL,
        setup: &[],
    },
};
