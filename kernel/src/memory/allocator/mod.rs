// 内核内存分配子系统
//
// 门户分配器 (portal) 作为 #[global_allocator]，在启动阶段通过 trait object
// 委托给不同后端。初始化顺序：
//   1. bump::init() — 标记 bump 可用内存区域
//   2. portal 切换到 bump trait object
//   3. page — 委托给 frame，在 memory::init() 中按需分配；无独立初始化
//
// bitmap — 通用位图分配器（编号空间连续区间：VA 窗口 / ASID），无独立初始化
// （位图首次使用时惰性分配），见 work::unit::space / memory::manager::asid。

use core::ptr::NonNull;

use fack::prelude::Error;

pub mod bitmap;
pub mod block;
pub mod bump;
/// 护栏层（in-path 运行时不变量检查）：checker（链断言）/ banker（页金库）/
/// ledger（活块账本）/ audit（核查）。
pub mod fence;
pub mod frame;
pub mod hybrid;
pub mod portal;

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
/// 注入物理内存池区域并完成 bump → hybrid 自举。
///
/// # Safety
///
/// 必须在 `main` 早期调用**恰好一次**，在任何堆分配之前。
/// 调用时 MMU 尚未启用，使用裸物理地址。
///
/// # Errors
///
/// 任一后端初始化失败（bump / hybrid 的错误原样传播，已在对应模块附加上下文）。
pub fn init() -> InitResult<()> {
    bump::init()?;
    log::debug!("bump");
    portal::switch(bump::allocator());

    hybrid::init()?;
    log::debug!("hybrid");
    portal::switch(hybrid::allocator());
    Ok(())
}
