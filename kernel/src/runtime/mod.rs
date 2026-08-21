// 运行时基础设施 — 陷阱分发、时钟、任务上下文与诊断停机
//
// 子模块：
//   clock      — 单调时钟（Duration 边界）：时间源 only（读时/换算/tick 基准）
//   timer      — 计时触发：deadline 注册表 + 武装/WFI 宿标 + tick 计数；依赖 clock
//   context    — TrapContext 帧（trap ABI，汇编与空间构建共用）
//   diagnose   — 诊断/监控族：watch/scene/trace/halt（看护、事件、现场、停机；见其 mod.rs）
//   trampoline — 陷阱进出汇编（__alltraps/__restore）与物理页地址
//   trap       — stvec 接线、内核帧元数据、scause 分发、SBI 定时器武装
//   envcall    — 用户态环境调用 ABI（RISC-V "Environment Call"，dispatch 经 trap 分发）
//
// 接线顺序：unit::init（构建内核空间、映射 TRAMPOLINE / 内核帧、封包 KERNEL_TEAM）→ clock::init（timebase 注入）→
// trap::init（stvec、sscratch、SIE）→ 任务化调度（S-timer 抢占）。

pub mod clock;
pub mod context;
pub mod diagnose;
pub mod envcall;
pub mod timer;
pub mod trampoline;
pub mod trap;


