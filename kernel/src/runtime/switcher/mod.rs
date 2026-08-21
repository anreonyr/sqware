//! switcher — 陷阱/上下文切换器：S-mode 进出内核态的全部机械
//!
//! 组合（同属「切换」责任面，相互咬合）：
//!   trap       — stvec 接线、内核帧元数据、scause 分发、SBI 定时器武装（trap_handler 入口）
//!   trampoline — 陷阱进出汇编（__alltraps/__restore）、per-hart trap 栈、tp 重建
//!   context    — TrapContext/Gprs 帧（trap ABI，汇编与空间构建共用）
//!   envcall    — 用户态环境调用 ABI 分发（trap_handler 的 UserEnvCall 分支）
//!
//! 调用链：用户态 ecall/中断 → trap（保存进 context 帧，跑在 trampoline 的 trap 栈上）
//! → envcall::dispatch（U 态 ecall）→ restore 回用户态。
pub mod context;
pub mod envcall;
pub mod trampoline;
pub mod trap;
