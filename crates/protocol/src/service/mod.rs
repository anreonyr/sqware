//! service — **那几台"服务"**（独立域）在**协议这一侧**的目录。
//!
//! **照实记（层六·3 的协议侧那一半）**：实现侧（`programs/src/system/` → `programs/src/service/`）
//! 先搬了 `hub` / `coalition` / `principal` 三台；协议侧对应的三个目录（`coalition` / `operator` /
//! `principal`）跟着搬——**量过**：`coalition` 25 处引用、`principal` 37 处、`operator` 144 处
//! ⇒ **一次搬一个**：`coalition`（第一刀）·`principal`（第二刀）……
//!
//! **协议侧少一个**：`hub` 没有对应目录（设备账那几面住 `system/supply`）。

pub mod coalition;
pub mod principal;
