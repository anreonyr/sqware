// 运行时模块 — 时间域、切换器（陷阱/上下文切换）、诊断族
//
// 子模块：
//   chrono     — 时间域：clock（时钟源：init/now/Instant/换算）+ timer（计时触发：tick/tock/drain）
//   switcher   — 陷阱/上下文切换器：trap（scause 分发/定时器武装）、trampoline（asm 进出/
//                trap 栈）、context（TrapContext 帧 ABI）、envcall（U 态环境调用分发；见其 mod.rs）
//   diagnose   — 诊断/监控族：watch/scene/trace/halt（看护、事件、现场、停机；见其 mod.rs）
//
// 接线顺序：unit::init（构建内核空间、映射 TRAMPOLINE / 内核帧、封包 KERNEL_TEAM）→
// chrono::clock::init（timebase 注入）→ switcher::trap::init（stvec、sscratch、SIE）→ 任务化调度。

pub mod chrono;
pub mod diagnose;
pub mod switcher;
