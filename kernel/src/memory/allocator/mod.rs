// 内核内存分配子系统
//
// 门户分配器 (portal) 作为 #[global_allocator]，以**无锁原子后端模式**（Backend：
// bump / hybrid / spare，见 portal.rs）在不同阶段分派。初始化顺序：
//   1. bump::init() — 标记 bump 可用内存区域
//   2. portal::switch(Backend::Bump) — 门户切到 bump（boot 单核，store 安全）
//   3. hybrid::init() — 运行时主堆后端（block + frame）
//   4. portal::switch(Backend::Hybrid)
//   5. spare::init() — 经 hybrid 一次整块分配诊断预算成后备仓（页级锁定，
//      绝不回收再分发，见 spare.rs；panic 现场唯一可信的分配源）
//
// bitmap — 通用位图分配器（编号空间连续区间：VA 窗口 / ASID），无独立初始化
// （位图首次使用时惰性分配），见 work::unit::space / memory::manager::asid。

use core::ptr::NonNull;

use fack::prelude::Error;

use crate::memory::manager::addr::AtomicPhysAddr;

pub mod bitmap;
pub mod block;
pub mod bump;
/// 护栏层（in-path 运行时不变量检查）：checker（链断言）/ banker（页金库）/
/// ledger（活块账本）/ audit（核查）。
pub mod fence;
pub mod frame;
pub mod hybrid;
pub mod portal;
/// 后备仓（日志 + panic 打印专用，从 bump carve，崩溃现场唯一可信分配源）。
pub mod spare;

/// 分配器初始化错误 — 与 `erra::Error<InitError>` 配对使用（见 [`InitResult`]）。
#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
#[allow(unused)]
pub enum InitError {
    /// 设备树未配置空闲内存区（`machine.free.size == 0`）。
    #[error("no free memory region configured")]
    NoFreeMemory,
    /// 初始化期间元数据分配失败（bump 池耗尽）。
    #[error("memory allocation failed while initializing allocator")]
    OutOfMemory,
    /// 空闲区不足一页，无法建立 frame 元数据。
    #[error("no free physical frames available")]
    NoFreeFrames,
    /// 设备树未报告任何 hart。
    #[error("no harts reported")]
    NoHarts,
    /// 分配器已被初始化（重复调用 init）。
    #[error("allocator already initialized")]
    AlreadyInitialized,
}

/// 分配器初始化结果 — `erra::Error<InitError>` 附加调用点上下文。
pub type InitResult<T> = erra::Result<T, InitError>;

struct Link {
    prev: Option<NonNull<Link>>,
    next: Option<NonNull<Link>>,
}

impl Link {
    fn new(prev: Option<NonNull<Link>>, next: Option<NonNull<Link>>) -> Self {
        Self { prev, next }
    }
}

/// 初始化内存子系统。
///
/// 注入物理内存池区域并完成 bump → spare → hybrid 三级自举。spare 容量**不
/// 显式注入**：内部按 `machine::hart_count()` 经诊断预算自行推导
/// （diagnose::budget::spare_budget = trace 环形常驻 + panic 打印峰值；与 bump
/// 读 `machine::info().free` 同一查源习惯）——在 hybrid 前从 bump carve，
/// frame/block 永不触碰。
///
/// # Safety
///
/// 必须在 `main` 早期调用**恰好一次**，在任何堆分配之前。
/// 调用时 MMU 尚未启用，使用裸物理地址。
///
/// # Errors
///
/// 任一后端初始化失败（bump / spare / hybrid 的错误原样传播，已在对应模块附加上下文）。
pub fn init() -> InitResult<()> {
    bump::init()?;
    portal::switch(portal::Backend::Bump);

    hybrid::init()?;
    portal::switch(portal::Backend::Hybrid);
    spare::init()?;
    Ok(())
}
