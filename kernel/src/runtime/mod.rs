// 运行时基础设施 — trap trampoline、TrapContext 帧、时钟、halt 处理器与陷阱分发
//
// 子模块：
//   clock      — 单调时钟（Duration 边界）：时间源 only（读时/换算/tick 基准）
//   timer      — 计时触发：deadline 注册表 + 武装/WFI 宿标 + tick 计数；依赖 clock
//   context    — TrapContext 帧（trap ABI，汇编与空间构建共用）
//   halt       — 内核 panic 处理器（无锁直写控制台后停机）
//   trampoline — 陷阱进出汇编（__alltraps/__restore）与物理页地址
//   trap       — stvec 接线、内核帧元数据、scause 分发、SBI 定时器武装
//
// 接线顺序：manager::init（映射 TRAMPOLINE / 内核帧）→ clock::init（timebase 注入）→
// trap::init（stvec、sscratch、SIE）→ 任务化调度（S-timer 抢占）。

pub mod clock;
pub mod timer;
pub mod context;
pub mod halt;
pub mod trampoline;
pub mod trap;
pub mod trace;
pub mod scene;

pub use trap::init;
