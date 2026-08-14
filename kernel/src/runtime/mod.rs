// 运行时基础设施 — trap trampoline、TrapContext 帧、halt 处理器与陷阱分发
//
// 子模块：
//   context    — TrapContext 帧（trap ABI，汇编与空间构建共用）
//   halt       — 内核 panic 处理器（无锁直写控制台后停机）
//   trampoline — 陷阱进出汇编（__alltraps/__restore）与物理页地址
//   trap       — stvec 接线、内核帧元数据、scause 分发、SBI 定时器
//
// 接线顺序：manager::init（映射 TRAMPOLINE / 内核帧）→ trap::init（stvec、
// sscratch、SIE）。阶段 A：内核态陷阱链路（S-timer 自检）。

pub mod context;
pub mod halt;
pub mod trampoline;
pub mod trap;

pub use trap::init;
