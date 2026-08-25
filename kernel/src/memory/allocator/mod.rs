// 内核内存分配子系统：门户 (portal) 作为 #[global_allocator]，以无锁原子后端
// 模式（Backend：bump / hybrid / spare）在不同阶段分派。

use core::ptr::NonNull;

use fack::prelude::Error;

pub mod bitmap;
pub mod block;
pub mod bump;
/// 护栏层：checker / banker / ledger / audit。
pub mod fence;
pub mod frame;
pub mod hybrid;
pub mod interval;
pub mod portal;
/// 后备仓（日志 + panic 打印专用）。
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

/// 初始化内存子系统（bump → hybrid → spare 自举）。
///
/// # Safety
///
/// 必须在 `main` 早期调用**恰好一次**，在任何堆分配之前。
/// 调用时 MMU 尚未启用，使用裸物理地址。
///
/// # Errors
///
/// 任一后端初始化失败，错误原样传播。
pub fn init() -> InitResult<()> {
    bump::init()?;
    portal::switch(portal::Backend::Bump);

    hybrid::init()?;
    portal::switch(portal::Backend::Hybrid);
    spare::init()?;
    Ok(())
}
