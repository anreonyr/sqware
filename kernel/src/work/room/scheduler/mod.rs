// 指令调度（scheduler）— 多核任务调度：纯核心 + 按面对齐的适配层
//
// 文件夹结构（核心与适配分离，见 design-pipeline）：
//   mod.rs — 薄壳：仅声明子模块（本文件不装调度代码）
//   core.rs — 纯功能核心：per-hart 调度器结构、方法、全局表、取活/休眠/回收、
//              当前任务身份槽 ident()（自包含、可独立推理，不依赖任何具体调用方）
//   适配层各一文件（入口面：「取本核 → 转发核心方法」，不重复业务逻辑；纯转发
//   查询已并入核心，不再设查询门面）：
//     boot.rs  — boot 装配入口（init / idle）
//     task.rs  — 任务生成入队（push）
//     trap.rs  — 陷阱路径入口（run）
//     utask.rs — 用户任务面（envcall 服务：starve / park / reap / wait / wake / join，
//                以及死岛上的 wait_forever）
//     ktask.rs — 内核任务面（软陷阱服务）：`reap` 的唯一调用者是
//                `TaskBuilder::ktask_trampoline`，而它自己是死岛；`park` / `starve` /
//                `wait_forever` 三个 asm 面树内零调用者。整片处置见
//                `docs/audit-flying-wires.md` §D3。
//
// 术语：tick/tock 属计时域；调度域词族 = run/starve/park/reap/steal/rotate/prepare/
// seat/shed。
//
// 命名（同词分面，路径 + 签名区分）：`park` / `starve` / `reap` 的三个面里，**核心那一面
// 已经不存在**——**事件面**的 `Scheduler::{park, wait, reap}` 随「任务离开 running 槽」
// 整体移入 messenger，核心只留跨边界原语 `disown_and_install_next`。故现在是两个面：
// `utask::park`(用户面) / `ktask::park`(内核面，死岛)。
//
// **待裁的一处撞车**：核心的 WFI/取活入口仍叫 `wait()`（`core.rs`），而冻结表把
// `wait`/`wake` 这对词给了 messenger 的事件等待与唤醒。同一个词两个意思，正名待裁决。

pub mod boot;
pub mod core;
pub mod ktask;
pub mod task;
pub mod trap;
pub mod utask;
