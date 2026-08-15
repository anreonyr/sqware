use alloc::alloc::{AllocError, Allocator};
use core::ptr::NonNull;

use erra::ResultExt;

use crate::memory::allocator::{InitError, InitResult};
use crate::{lock::SpinLock, machine};

pub(crate) struct BumpAllocator {
    inner: SpinLock<Option<BumpInner>>,
}

impl BumpAllocator {
    const fn new() -> Self {
        Self {
            inner: SpinLock::new(None),
        }
    }

    pub fn init(&self) -> Result<(), InitError> {
        let mut guard = self.inner.lock();
        let mut inner = BumpInner::new(0, 0, 0);
        inner.init()?;
        guard.replace(inner);
        Ok(())
    }
}

struct BumpInner {
    used: usize,
    base: usize,
    edge: usize,
}

impl BumpInner {
    const fn new(base: usize, edge: usize, used: usize) -> Self {
        Self { base, edge, used }
    }

    fn init(&mut self) -> Result<(), InitError> {
        // 区域来自调用方注入的 memory::platform 配置（allocator::init 设置），
        // 不再引用内核链接符号 _bump_base——自包含。
        let m = machine::get();
        if m.free.size == 0 {
            return Err(InitError::NoFreeMemory);
        }
        self.base = m.free.base;
        self.edge = m.free.base + m.free.size;
        Ok(())
    }
}

unsafe impl Allocator for BumpAllocator {
    fn allocate(
        &self,
        layout: core::alloc::Layout,
    ) -> Result<core::ptr::NonNull<[u8]>, AllocError> {
        let mut guard = self.inner.lock();
        let inner = guard.as_mut().ok_or(AllocError)?;

        let frontier = (inner.base + inner.used) as *mut u8;
        let next = unsafe { frontier.add(frontier.align_offset(layout.align())) };

        if next.addr() + layout.size() > inner.edge {
            return Err(AllocError);
        }

        inner.used = next.addr() - inner.base + layout.size();
        Ok(NonNull::slice_from_raw_parts(
            NonNull::new(next).ok_or(AllocError)?,
            layout.size(),
        ))
    }

    unsafe fn deallocate(&self, _ptr: core::ptr::NonNull<u8>, _layout: core::alloc::Layout) {}
}

pub fn boundary() -> usize {
    let guard = BUMP_ALLOCATOR.inner.lock();
    let inner = guard.as_ref().expect("bump allocator not initialized");
    inner.edge
}
pub fn frontier() -> usize {
    let guard = BUMP_ALLOCATOR.inner.lock();
    let inner = guard.as_ref().expect("bump allocator not initialized");
    inner.base + inner.used
}

/// Bump 分配器实例 — 通过 PortalAllocator 的 trait object 间接调用。
pub(crate) static BUMP_ALLOCATOR: BumpAllocator = BumpAllocator::new();

/// 获取 bump 分配器的 `&'static dyn Allocator` 引用 — 供 PortalAllocator 使用。
pub fn allocator() -> &'static dyn Allocator {
    &BUMP_ALLOCATOR
}

/// 初始化 bump 分配器的内存区域。
///
/// 必须在 `main` 早期调用恰好一次，在任何堆分配之前。
///
/// # Errors
///
/// 机器未配置空闲内存区 → [`InitError::NoFreeMemory`]。
pub fn init() -> InitResult<()> {
    BUMP_ALLOCATOR
        .init()
        .annotate("initializing bump allocator")
}
