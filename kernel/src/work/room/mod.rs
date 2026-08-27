// 调度房间（room）— per-hart 调度核心与系统级生命周期。
//
//   conductor — 指令调度（纯核心 + 按面对齐的适配层，见 conductor/mod.rs）
//   tie       — 系统级生命周期（任务计数/全退出停机、休眠核位图/IPI 唤醒）

pub mod conductor;
pub(crate) mod tie;
