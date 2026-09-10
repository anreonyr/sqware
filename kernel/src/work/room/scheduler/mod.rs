// 指令调度（scheduler）— 多核任务调度：纯核心 + 两个入口面。
//
// 文件夹结构（核心与入口分离，见 design-pipeline）：
//   mod.rs  — 薄壳：仅声明子模块（本文件不装调度代码）
//   core/   — 纯功能核心（自包含、可独立推理，不依赖任何具体调用方）：
//     mod.rs   声明四子文件 + 重导出（入口面只经它触及核心）
//     hart.rs  本机调度器：容器（running + 就绪队列）、装槽 / 让位 / 轮转 / 续跑
//     ident.rs 本核身份槽 `Badge` + 身份读法 `ident()`
//     table.rs 全局表（per-hart 调度器数组）+ 名册 + 全机扫描 + 关机终末释放
//     fetch.rs 取活：跨核偷取 + WFI 休眠
//   入口面各一文件（「取本核 → 转发核心方法」，不重复业务逻辑）：
//     boot.rs  — boot 装配入口（init / idle）
//     trap.rs  — 陷阱路径入口（run：续跑 / 轮转 / 取活）
//
// **「任务面」两个文件已删除**（`task.rs` 放行入队、`utask.rs` envcall 服务面、
// `ktask.rs` 内核任务面）：它们的划分来自「用户任务 / 内核任务」二分，而内核任务
// 已不再支持（内核闭包任务的生产者 `TaskBuilder::closure` 与软陷阱装配
// `ktask_trampoline` 一并删除）——剩下的调用方直呼真正的归属：放行入队进 `core::launch`，
// 事件与退场进 [`crate::work::room::messenger`]，让出/取活进本核心的 `starve`/`fetch`。
//
// 术语：tick/tock 属计时域；调度域词族 = run/starve/park/reap/steal/rotate/prepare/
// seat/shed。这些词的**服务面现在只剩一处**：`park`/`reap` 的实现都在 messenger
// （任务「离开 running 槽」的状态机归它），本核心只留跨边界原语
// `swap`（取走 running + 装下一帧）与身份槽两态（`Badge::seat` 装 / `Badge::shed` 降级）。
//
// **一处曾记的撞车已结构性消解**（原「待裁」）：核心原先的 WFI/取活入口叫 `wait()`，
// 与冻结表给 messenger 的 `wait`/`wake` 同词。现在 `wait` 只是 `core/fetch.rs` 内部
// 私有的 WFI 步骤，对外入口是 `fetch`——同一个词不再有两个意思。名字未动（用户裁决）。

pub mod boot;
pub mod core;
pub mod trap;
