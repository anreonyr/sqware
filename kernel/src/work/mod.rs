// 任务（task）— 进程/线程模型与调度。
//
// 一个 Team 持有唯一 Space（共享地址空间），多个 Task 共享之。
//
//   unit   — 任务执行单元（space / gate / team / task / life / weak / loader / parser）
//   room   — 调度房间（scheduler / messenger / conductor）
//   mail   — 任务间通信（Hole 单槽邮路 / Pole 共享页视图 / Nole 权柄载体 / Tole 多路等待）

pub mod mail;
pub mod room;
pub mod unit;
