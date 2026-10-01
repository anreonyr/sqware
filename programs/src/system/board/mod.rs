//! board::实现侧 — **板那一台**、它那本客人账，与**说话的那一侧**（客侧那几手）。
//!
//! 板这份协议分三层住：**规范与记号**（正文、帧、失败域、那几格记号）住
//! `crates/protocol/src/system/board/`；**客侧**（[`client`]：从外面找上板的那几手）与
//! **实现方**（真在编排域里跑的那枚线程 + 它记的账）住这里（`programs/src/system/board/`）。
//!
//! **照实记（客侧原先住 protocol）**：按裁定「board 是编排域的**死信号传感器**，不是第五轴」，
//! `client.rs` 从 protocol 那一层退回实现侧——那里只留**形与记号**（见 [`client`] 的头注）。
//! 判据仍是同一条：**谁在说话**——从外面找上板的人（客人、装配者）用的一切是那一侧的接口；
//! 板自己怎么站住、怎么记账是实现。

//! # 照实记（这一族今天还剩什么、谁在靠它——**撤它那一刀的施工图**）
//!
//! 两层按先后分成两半，而**只有后一半还活着**：
//!
//! | 那一半 | 是什么 | 今天 |
//! |---|---|---|
//! | **死信号** | 装配者给每位要"存在信号"的台铸一条道（`gone-<名字>`）→ 板看见某位的门封印了就往里推一格 → 监督从组上醒来据此记账 | **整片退场**（两刀：监督那一趟改读内核那一格「它收尾了」；板这一侧的 `lane` 一片随后也退） |
//! | **待客那本账** | 客人自己开一条 `board::BERTH` 会话 → `enroll`（交入口、编名字）→ 板上记一格；`register` / `evict` 各答一格码 | **活着**（下表那几处是它的全部用户） |
//!
//! ## 谁在靠它（逐处点过名；撤它要一起动的四处）
//!
//! 一、**装配者那一侧的一条握手**（`board::bridge::attach`）：`presence: true` 的台**必须**
//! 自己开一条板会话——装配者按 `(本域, 板路)` 认领它交出来的那一枚，**没开会话的台装配当场
//! 报 `board:claim`**。故"这一台要不要板位"是**一对握手的两头**，不能只删一头
//! （`Relation::presence` 那一格就是这一头，它的头注里记着这件事）。
//!
//! 二、**读回话的客侧**（撤它们要改的是**验收读数**，不只是代码）：
//!
//! | 那一处 | 它拿回话做什么 |
//! |---|---|
//! | `harness/src/guest.rs` | `assert_eq!(reg, bcall::OK)`；`bye`（`evict` 那一格）也进读数 |
//! | `harness/src/passer.rs` | `assert_eq!(reg, bcall::OK)` |
//! | `harness/src/sleeper.rs` | `reg` 是它那一步的读数 |
//!
//! 三、**只开会话、不看回话的**（`let _ = …` / `let _board = …`）：`canonical`、
//! `driver::context`、`principal`、`coalition`（四处的 `Session::open(sire, BERTH)`）——
//! 它们开会话的理由就是上面那条握手（不开装配就红）。
//!
//! 四、**记号那一份要各归各家**：`ENTRY_MARK`（域的服务入口，`driver::{uart,rtc}` 与
//! `router::adapt::boot` 各有一处读者）、`TIP_MARK` 与 `LINK`（**`operator` 那一族也在用**，
//! 它 `pub use` 的就是这一份）——它们今天借住本模块，撤族时要落到"那一格真正的主人"那里。
//!
//! ## 这一族里已经点过的死格（都退过场，列在这里免得下一刀再找一遍）
//!
//! · `Unregister` / `Lookup` 两问（没有生产者）· 两张形状里那张 `Name` 帧 · `LANE_PREFIX`
//! 与 `gone-` 那一族 · `Tip` 帧里的 `name` 那一格 · `Guest::lane` / `note_lane` / `take_lane`
//! · `board.rs` 里那本 `Lanes`。

pub mod bridge;
pub mod client;
// **照实记（`core` 是残枝那一刀从 protocol 搬来的）**：板那本账原先住
// `crates/protocol/src/system/board/core.rs`——读者只有本域的持板线程，故回这里。
pub mod core;
pub mod server;
