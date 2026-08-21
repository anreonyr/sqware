//! diagnose — 诊断/监控族：看护、事件、现场、停机（崩溃链路的同一责任面）。
//!
//! 组合（相互咬合）：
//!   watch  — 值班看护：抓「活着但没在爬」的失速/锁相持（判据全原子、无锁不分配）
//!   trace  — 事件环形缓冲 + 宿主镜像（semihosting fs 导出 JSON Lines），崩溃反推依据
//!   scene  — 崩溃现场转储（GPR/CSR/回溯符号化，回答「崩在哪」）
//!   halt   — 停机决策：panic 处理器（抢占报警源、广播停其它核、诊断输出）
//!
//! 崩溃链路：watch 上报 / 断言失败 → panic → halt 拉警报 → trace 记 Panic 事件 →
//! scene 转现场；读取侧（panic_dump）只在报警核、halt 已让其它核停写后运行。
pub mod halt;
pub mod scene;
pub mod trace;
pub mod watch;
