//! System 的**设备面**：受信组成在自己这一域里持有的那套控制。
//!
//! 今天这里只有一件东西：[`platform`]——一枚真实 RISC-V IOMMU 的**独立停机权威**。
//! 它把 `retention::Platform` 的 `synchronize` / `isolate` 落到真寄存器上，
//! 不向任何设备服务要东西；它与 [`crate::driver::iommu`] 那套机制（寄存器格 /
//! 上下文 / 表 / 队列）共用同一份口径，不另立第二套寄存器读法。
//!
//! 还没接上的那一半（产品 DMA 服务与它的工作页保留/发布）在下一次提交：
//! 平台已经有了，缺的是把它挂进 `Holds::attach` 并真发布页的那一份管理者。

pub mod platform;
