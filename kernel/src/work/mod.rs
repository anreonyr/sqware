// 任务（task）— 线程模型：Team（进程）→ Task（线程）两层
//
// 一个 Team 持有唯一 Space（共享地址空间），多个 Task 共享之；每个 Task 持有
// 自己的 trap 帧（Frame 窗口分配，任意 VA——alltraps/restore 经帧内 self_va
// 定位）。由 S-timer 抢占 + envcall 驱动切换。切换完全走 trap 链路——
// trap_handler 返回下一任务帧 → restore 切 satp + sret，无独立切换汇编。
//
// 子模块：
//   scheduler — per-hart 调度核心（Scheduler/每核表/切换/回收/steal）
//   tie       — 系统级生命周期（任务计数/全退出停机、休眠核位图/IPI 唤醒）
//   team      — 团队容器（Team/TeamBuilder/kernel 单例）
//   loader    — 程序装载（ELF → Space durable）
//   task      — 线程单元（Task/TaskBuilder）
//   envcall   — 用户态环境调用 ABI（RISC-V "Environment Call"）
//
// 启动编排（调度器 init/自测 → Team/Task 构建 → HSM 副核 → 首任务）在 boot.rs。

pub mod envcall;
pub(crate) mod loader;
pub(crate) mod parser;
pub mod scheduler;
pub(crate) mod task;
pub(crate) mod team;
pub(crate) mod tie;

use crate::memory::manager::addr::VirtAddr;

/// 用户程序加载基址。
pub const USER_TEXT_BASE: VirtAddr = VirtAddr::from_raw(0x1_0000);

// 常用 API 收敛到 work::；只 re-export 外部引用者（envcall/trap）。
pub use scheduler::run;
