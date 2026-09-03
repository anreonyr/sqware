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
//     utask.rs — 用户任务面（envcall 服务：park / starve / reap）
//     ktask.rs — 内核任务面（软陷阱服务：park / starve / reap）
//
// 术语：tick/tock 属计时域；调度域词族 = run/starve/park/reap/steal/wait/
// rotate/prepare/mount/unpark。命名三面同词（park/starve/reap），路径 +
// 签名区分——`Scheduler::park`(核心方法，原 Conductor::park) / `utask::park`(用户面) /
// `ktask::park`(内核面)。

pub mod boot;
pub mod core;
pub mod ktask;
pub mod task;
pub mod trap;
pub mod utask;
