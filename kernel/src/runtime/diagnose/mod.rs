//! diagnose — 诊断族：事件、报告、现场、停机、导出（崩溃链路的同一责任面）。

/// 执行历史的投影核心（帧 + 地址语义，零分配）。
pub mod backtrace;
#[cfg(feature = "semihosting")]
pub mod export;
/// 领域无关的执行链投影引擎（栈采样 + 链投影）。
pub mod frame;
pub mod halt;
/// IPI 自检（debug 档）：一记 SBI IPI 到底能不能把 WFI 里的核叫醒。
#[cfg(debug_assertions)]
pub mod ipi;
/// 退场的账：哪一台、什么结局、它走时留的话（只记不判）。
pub mod ledger;
/// 表格渲染适配：stanza 定宽栅格（列宽自适应）；报告印发。
pub mod render;
/// 诊断报告核心（段落 + 行；成册/清空生命周期）。
pub mod report;
pub mod scene;
pub mod trace;
