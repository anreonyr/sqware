// 任务（task）— 进程/线程模型与调度。
//
// 一个 Team 持有唯一 Space（共享地址空间），多个 Task 共享之。
//
//   unit   — 任务执行单元（space/team/task/loader/parser/elftable）
//   room   — 调度房间（scheduler + tie）
//   mail   — 任务间通信（port 内核邮路 / ring 共享内存邮路）

pub mod mail;
pub mod room;
pub mod unit;
