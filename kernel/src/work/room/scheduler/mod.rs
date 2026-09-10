// 指令调度（scheduler）— 多核任务调度：纯核心 + 两个入口面。
//
// 文件夹结构（核心与入口分离，见 design-pipeline）：
//   mod.rs — 薄壳：仅声明子模块（本文件不装调度代码）
//   core.rs — 纯功能核心：per-hart 调度器结构、方法、全局表（含放行入队 `push`）、
//              取活/休眠/回收、当前任务身份槽 ident()（自包含、可独立推理，不依赖
//              任何具体调用方）
//   入口面各一文件（「取本核 → 转发核心方法」，不重复业务逻辑）：
//     boot.rs  — boot 装配入口（init / idle）
//     trap.rs  — 陷阱路径入口（run：取活 / 轮转 / 续跑）
//
// **「任务面」两个文件已删除**（`task.rs` 放行入队、`utask.rs` envcall 服务面、
// `ktask.rs` 内核任务面）：它们的划分来自「用户任务 / 内核任务」二分，而内核任务
// 已不再支持（内核闭包任务的生产者 `TaskBuilder::closure` 与软陷阱装配
// `ktask_trampoline` 一并删除）——剩下的调用方直呼真正的归属：放行入队进 `core::push`，
// 事件与退场进 [`crate::work::room::messenger`]，让出/取活进本核心的 `starve`/`run`。
//
// 术语：tick/tock 属计时域；调度域词族 = run/starve/park/reap/steal/rotate/prepare/
// seat/shed。这些词的**服务面现在只剩一处**：`park`/`reap` 的实现都在 messenger
// （任务「离开 running 槽」的状态机归它），本核心只留跨边界原语
// `disown_and_install_next` 与槽位两态（`seat` 装 / `shed` 降级）。
//
// **待裁的一处撞车**：核心的 WFI/取活入口仍叫 `wait()`（`core.rs`），而冻结表把
// `wait`/`wake` 这对词给了 messenger 的事件等待与唤醒。同一个词两个意思，正名待裁决。

pub mod boot;
pub mod core;
pub mod trap;
