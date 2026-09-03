// 调度房间（room）— per-hart 调度核心与系统级生命周期。
//
//   scheduler  — 指令调度（纯核心 + 按面对齐的适配层，见 scheduler/mod.rs）
//   messenger  — 事件队列（park / wait-by-key / wake / reap / clear）
//   conductor  — 多核协调（任务计数/全退出停机、休眠核位图/IPI 唤醒）

pub(crate) mod conductor;
pub mod messenger;
pub mod scheduler;
