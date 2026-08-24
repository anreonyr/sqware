//! diagnose — 诊断族：事件、报告、现场、停机、导出（崩溃链路的同一责任面）。

#[cfg(feature = "semihosting")]
pub mod export;
/// 表格渲染适配：stanza 定宽栅格（列宽自适应）；报告印发。
pub mod render;
/// 诊断报告核心（段落 + 行；成册/清空生命周期）。
pub mod report;
pub mod halt;
pub mod scene;
pub mod trace;