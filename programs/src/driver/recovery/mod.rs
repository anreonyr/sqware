//! 设备恢复那一台（β）：**独立常驻**的设备专用恢复方。
//!
//! 它只做本设备的三手：`attach`（发布前取得精确 BAR 恢复映射、再把子授权交给执行方）、
//! `stop`（业务 CPU 控制切断后执行本设备 reset/drain 并回读）、`detach`（停链完成后放下）。
//! 它不解释 IOMMU、不碰 ECAM、不构造任务、也不保存总设备账——那些各归各域。
//!
//! - [`api`] / [`core`] / [`serve`]：门上的问与答、本域核心与常驻面；
//! - [`device`]：一号绑定上的恢复事实。

pub mod api;
pub mod core;
pub mod device;
pub mod serve;
mod attach_request;
