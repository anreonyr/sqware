// 调度房间（room）— per-hart 调度核心与系统级生命周期
//
// room 承载「谁在跑、何时切换、何时刻系统停机」的运行层状态，区别于 unit
// （地址空间/团队/线程的静态容器）。子模块：
//   scheduler — per-hart 调度核心（Scheduler/每核表/切换/回收/steal）
//   tie       — 系统级生命周期（任务计数/全退出停机、休眠核位图/IPI 唤醒）

pub mod scheduler;
pub(crate) mod tie;
