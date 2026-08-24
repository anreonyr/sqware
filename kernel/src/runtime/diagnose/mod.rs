//! diagnose — 诊断族：事件、报告、现场、停机、导出（崩溃链路的同一责任面）。
//!
//! 组合（相互咬合）：
//!   report — 诊断报告核心：信息收集模块化、一次印发全部信息（段落 + 行，
//!           投稿方直写，成册后两出口——控制台表格 / 宿主 JSON）
//!   export — 宿主导出：单文件 JSON 流（事件行实时 + 报告快照整档）
//!   trace  — 事件环形缓冲（运行时设施）+ 宿主镜像（事件行实时导出）
//!   scene  — 崩溃现场转储（GPR/CSR/回溯符号化，回答「崩在哪」）
//!   halt   — 停机决策：panic 处理器（抢占报警源、广播停其它核、诊断输出）
//!
//! 崩溃链路：断言失败 → panic → halt 拉警报 → trace 记 Panic 事件 →
//! scene/trace 组稿进报告 → seal → render（控制台）+ export（宿主）。
#[cfg(feature = "semihosting")]
pub mod export;
/// 表格渲染适配：stanza 定宽栅格（列宽自适应）；报告印发 + 迁移期收集器。
pub mod render;
/// 诊断报告核心（段落 + 行；成册/清空生命周期）。
pub mod report;
pub mod halt;
pub mod scene;
pub mod trace;