// 内核分配器门户 — 无锁后端模式分派（#[global_allocator]）
//
// 后端锁定在编译期已知的三种实现（bump / hybrid / spare），门户只存一个原子
// 判别位（AtomicU8），alloc 路径一次 Acquire 读即分派——**不取任何锁**：
//   · boot 切换（Bump → Hybrid）发生在单核期，store 天然安全；
//   · 崩溃切换（→ Spare）发生在报警核，原子 store 无锁——panic 现场即使正有
//     他核持主堆锁，也绝不会被本切换卡死。
// 跨核分配互斥全部由后端自带锁负责（bump / block-inner / frame-inner / tally）。
//
// 注意：portal 解锁后，block 的簿记表读（own）改由 tally 自锁串行（见 block.rs——
// 这是从"portal 锁兼任跨池闸门"迁移过来的唯一承重点）。

use core::alloc::{AllocError, Allocator, Layout, GlobalAllocator};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU8, Ordering};

use super::{bump, hybrid, spare};

/// 后端模式：0 = 未装配（分配返回 Err）；1..=3 = 对应后端。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Backend {
    Bump = 1,
    Hybrid = 2,
    Spare = 3,
}

/// 当前后端判别位（0 = 未装配）。
static BACKEND: AtomicU8 = AtomicU8::new(0);

fn backend() -> Option<&'static dyn Allocator> {
    match BACKEND.load(Ordering::Acquire) {
        b if b == Backend::Bump as u8 => Some(bump::allocator()),
        b if b == Backend::Hybrid as u8 => Some(hybrid::allocator()),
        b if b == Backend::Spare as u8 => Some(spare::allocator()),
        _ => None,
    }
}

pub struct PortalAllocator;

unsafe impl Sync for PortalAllocator {}

unsafe impl GlobalAllocator for PortalAllocator {}

unsafe impl Allocator for PortalAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        backend().ok_or(AllocError)?.allocate(layout)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe {
            if let Some(allocator) = backend() {
                allocator.deallocate(ptr, layout);
            }
        }
    }
}

/// 切换后端（无锁原子 store）。boot 单核 / 崩溃报警核单核调用；Release 发布，
/// 与 alloc 路径的 Acquire 读配对，store 前写入对后续分配可见。
pub fn switch(backend: Backend) {
    BACKEND.store(backend as u8, Ordering::Release);
}

#[global_allocator]
pub static PORTAL_ALLOCATOR: PortalAllocator = PortalAllocator;