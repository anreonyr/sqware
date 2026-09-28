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
//! ——它们只跟树说话，不需要任何驱动；而"要读树"的客人离扳机越近窗口越窄（那条窗口的现状与
//! 残余见下面第三件）。
//!
//! **照实记（重量的那一遍：「收场口令喂得太早」其实是三件事，不是一件）**——同一份提交镜像、
//! 直接起 QEMU（绕开 `scripts/boot.nu`）量的：
//!
//! 1. **起机那一瞬喂进去的那一发落不进去**：单发一行落在 t=0 ⇒ 3/4 **不停机**（rc=124、没有停机
//!    行，而 14~16 条读数都在）；同一份镜像 ≥1 s 单发 ⇒ **28/28 停机**。症状是「没有停机行」，
//!    **不是**「随机某几台没有读数」。不喂输入时机器不会自己停，读数**全部**到齐（0/10 停机）。
//! 2. **装配期折一条**（`system: assemble`，牺牲者每次不同：`board:claim` / `start failed`）：
//!    一折之后**其后台全部没有读数**——上一版记成「喂太早 ⇒ 随机某几台没读数」的就是它。它是
//!    **另一件**：只在起机那一瞬有输入的跑里见过（2/6 对 0/38），样本小、方向一致。
//! 3. **`probe-operator-gate` 那一条读数**在那些跑里丢过 20/39，而它**不是**"推迟喂入"治得了的
//!    （**完全不喂**的跑里也丢：30 s 窗口 2/6）。查下去是**两个真缺陷**，都已收掉：
//!    ① 探针改成"数格子"之后当场数到 **8**——`/sys/operator` 底下**多出一格也叫 `operator` 的
//!       自己**（那段目录也铸了一枚孔、也被落了一格），与"它自己不是一格"正相反；旧写法**按名字
//!       数**故一直没显形。根因与那一刀见 `programs/src/system/mod.rs::Assembly::mount_control`
//!       那一节（四份 `mount.rs` 已随回炉收成一处，故照实记也归了那一格）。
//!    ② 那一族的问话**推得进去**这件事没人保（`protocol` 的 `client.rs::call`：推是
//!       `Send(.., Wait::Forever)`，孔是单槽）⇒ 问得越多越可能等在门外、被扳机扑杀在
//!       "绿也没有红也没有"那一格里。探针从"`list` ＋ 七问名"收成"一问数格子"，走不通就
//!       **当场红**（`harness/src/probe_operator_gate.rs` 的 `count_under` 与 `WAIT_MS`）。
//!
//! 收完之后 `scripts/boot.nu` 默认档 **19 跑里 17 全绿**，剩下那两跑是**同一件事的残余**：树那台
//! 服务只有**一枚线程**，一位客人的问话可以排在别人后头，而停机扳机不等它（残余率 ~10%，症状仍是
//! "没有那条读数、也没有红"）。要根治得让扳机**等读数**（喂入口令由"读数到齐"驱动），不是再猜
//! 一个秒数——那一刀没在这轮里动。
//!
//! 故 `scripts/boot.nu` 今天**两档都先起稳再喂**（单行档也一样，起稳后每 2 s 重喂；默认 6 s 是
//! **读数走完**的下限，量法见那个文件），而攒不住 stdin（`mktemp` 失败）时**出声**，不再静默地
//! 一个字节都不喂。

use crate::program::{Demand, Died, Identity, Program, Relation, Spot};

/// 它死在起手哪一步。
pub const E_CANONICAL: Died = 24;

pub static PROGRAM: Program = Program {
    identity: Identity {
        name: "canonical",
        spot: Spot::Console,
        scenes: &["root", "product"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        order: Some(21),
        presence: true,
        operator: true,
        bind: true,
        ..Relation::DEFAULT
    },
    demand: Demand {
        died: E_CANONICAL,
        ..Demand::DEFAULT
    },
};
