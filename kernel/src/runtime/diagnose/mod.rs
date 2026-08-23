//! diagnose — 诊断族：事件、现场、停机、导出（崩溃链路的同一责任面）。
//!
//! 组合（相互咬合）：
//!   export — 宿主导出：JSON Lines 单文件（sqware-diagnose.jsonl），trace/scene/halt
//!           的结构化记录统一经此落盘（终端纯文本归 console.rs）
//!   trace  — 事件环形缓冲 + 宿主镜像（经 export 导出），崩溃反推依据
//!   scene  — 崩溃现场转储（GPR/CSR/回溯符号化，回答「崩在哪」）
//!   halt   — 停机决策：panic 处理器（抢占报警源、广播停其它核、诊断输出）
//!
//! 崩溃链路：断言失败 → panic → halt 拉警报 → trace 记 Panic 事件 →
//! scene 转现场；读取侧（panic_dump）只在报警核、halt 已让其它核停写后运行。
#[cfg(feature = "semihosting")]
pub mod export;
/// 预算政策：spare 仓容量（trace 环形常驻 + panic 打印峰值）单源。
pub mod budget;
pub mod halt;
pub mod scene;
pub mod trace;