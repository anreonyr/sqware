// 内核内存分配子系统
//
// 门户分配器 (portal) 作为 #[global_allocator]，在启动阶段通过 trait object
// 委托给不同后端。初始化顺序：
//   1. bump::init() — 标记 bump 可用内存区域
//   2. portal 切换到 bump trait object
//   3. page — 委托给 frame，在 memory::init() 中按需分配；无独立初始化
//
// bitmap — 通用位图分配器（编号空间连续区间：VA 窗口 / ASID），无独立初始化
// （位图首次使用时惰性分配），见 memory::manager::space / asid。

use core::ptr::NonNull;

pub mod bitmap;
pub mod block;
pub mod bump;
pub mod frame;
pub mod hybrid;
pub mod portal;

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
pub fn init() {
    bump::init();
    log::debug!("bump");
    portal::switch(bump::allocator());

    hybrid::init();
    log::debug!("hybird");
    portal::switch(hybrid::allocator());
}
