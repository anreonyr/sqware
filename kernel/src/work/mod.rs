// 任务（task）— 进程/线程模型与调度
//
// 一个 Team 持有唯一 Space（共享地址空间），多个 Task 共享之（见 work::unit）。
// 由 S-timer 抢占 + envcall 驱动切换。切换完全走 trap 链路——
// trap_handler 返回下一任务帧 → restore 切 satp + sret，无独立切换汇编。
//
// 子模块：
//   unit      — 任务执行单元（space/team/task/loader/parser/elftable，静态容器）
//   room      — 调度房间（scheduler per-hart 调度核心 + tie 系统级生命周期）
// 注：envcall（用户态环境调用 ABI）已收编进 runtime。
//
// 启动编排（调度器 init/自测 → Team/Task 构建 → HSM 副核 → 首任务）在 boot.rs。

pub mod room;
pub mod unit;

use crate::memory::manager::addr::VirtAddr;

/// 用户程序加载基址。
pub const USER_TEXT_BASE: VirtAddr = VirtAddr::from_raw(0x1_0000);

// 常用 API 收敛到 work::；只 re-export 外部引用者（trap）。
pub use room::scheduler::run;
