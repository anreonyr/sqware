//! service — **那几台"服务"**（独立域）那一层的目录。
//!
//! **照实记（层六·3：`system/ → service/`，一次一个域）**：`system/` 那一层原先混着两件事——
//! **装配机器**（`control/`、`machine`、`source`、`schedule`…）与**那几台服务本身**
//! （`operator` / `principal` / `coalition` / `hub`）。这一层把"服务"那半边收进 `service/`，
//! **一次搬一个域**（量过：`operator` 178 处引用 / `principal` 52 / `coalition` 32 / `hub` 4
//! ⇒ 先搬最小的那一个，其余逐个来）。
//!
//! **搬过的**：`hub`（设备账那一台）·`coalition`（盟册那一台）。**`system/` 里还剩** `operator`（178 处
//! 引用）与 `principal`（52 处）——各是一刀（量过的数在 [`system`](crate::system) 那一侧那几份头注里）。

pub mod coalition;
pub mod hub;
