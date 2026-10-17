//! System 的**设备面**：受信组成在自己这一域里持有的那套控制。
//!
//! - [`platform`]：一枚真实 RISC-V IOMMU 的**独立停机权威**——它把 `resource::retention::Platform`
//!   的 `synchronize` / `isolate` 落到真寄存器上，不向任何设备服务要东西；它与
//!   [`crate::driver::iommu`] 那套机制（寄存器格 / 上下文 / 表 / 队列）共用同一份口径。
//! - [`dma`]：产品那一份设备管理者——保留/发布工作页，把精确一根 BAR 交给一台跨 Team 的
//!   端点服务，并在它死后自己收口；起手与停机都是按帧推进的。
//! - [`path`] / [`started`] / [`start`] / [`stage`] / [`stop`] / [`step`]：上面两位要用的
//!   枚举（一型一文件）；[`endpoint_drain`] 是端点侧收口要的那几个坐标与能力表。
//! - [`fault`]：只给目标景用的一层受控注入。

pub mod dma;
pub mod endpoint_drain;
pub mod fault;
pub mod path;
pub mod platform;
mod stage;
mod start;
pub mod started;
mod step;
mod stop;
