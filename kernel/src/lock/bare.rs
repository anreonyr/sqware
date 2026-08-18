// BareLock — 不关中断的自旋锁，仅任务上下文
//
// 与 SpinLock 相同的互斥语义，但获取时不关闭 sstatus.SIE，因此中断延迟更低。
// 代价：绝不能从中断上下文获取——否则"任务持锁 → 中断抢占 → 中断路径争同一锁"
// 会死锁。为在类型层面强制这一约束，lock() 标记为 unsafe fn。
//
// 适用场景：只在启动期或任务上下文访问、从不被中断处理程序碰的共享状态。

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::dep;

pub struct BareLock<T: ?Sized> {
    locked: AtomicBool,
    /// 当前持有者调用点（返回地址；0 = 未持有）——重入检测与死锁溯源用
    holder_pc: AtomicUsize,
    data: UnsafeCell<T>,
}

// SAFETY: 同一时刻只有一个 guard 持有 &mut T；guard !Send，锁在本 hart 释放。
unsafe impl<T: ?Sized> Sync for BareLock<T> {}

/// 锁守卫 — 持有期间通过 Deref/DerefMut 访问受保护数据，析构时释放锁。
pub struct BareLockGuard<'a, T: ?Sized> {
    lock: &'a BareLock<T>,
    // *const () 既不 Send 也不 Sync：强制 guard 在本 hart 上释放
    _not_send: PhantomData<*const ()>,
}

impl<T> BareLock<T> {
    #[allow(dead_code)] // 当前无用户，锁体系预留
    pub const fn new(val: T) -> Self {
        BareLock {
            locked: AtomicBool::new(false),
            holder_pc: AtomicUsize::new(0),
            data: UnsafeCell::new(val),
        }
    }
}

impl<T: ?Sized> BareLock<T> {
    /// 获取锁，返回守卫。不关中断。
    ///
    /// # Safety
    ///
    /// 调用者必须保证绝不从中断上下文争用此锁，否则同 hart 中断重入会死锁。
    /// 仅可用于启动期或纯任务上下文的共享状态。
    #[allow(dead_code)] // 当前无用户，锁体系预留
    #[inline(never)] // 保证入口读到的 ra 是调用者返回地址（内联会破坏）
    pub unsafe fn lock(&self) -> BareLockGuard<'_, T> {
        // 入口第一件事：捕获调用者返回地址（任何函数调用都会覆盖 ra）
        let caller = dep::read_ra();
        // Acquire：保证后续读取看到之前写入的完整状态
        if self.locked.swap(true, Ordering::Acquire) {
            // 单 hart 下锁已被持有必是同执行流重入（不关中断下亦然）——报告后 panic
            let holder = self.holder_pc.load(Ordering::Relaxed);
            dep::report(
                "barelock",
                "recursive acquisition",
                self as *const Self as *const () as usize,
                holder,
                caller,
            );
        }
        self.holder_pc.store(caller, Ordering::Relaxed);

        BareLockGuard {
            lock: self,
            _not_send: PhantomData,
        }
    }

    /// 尝试获取锁，成功返回守卫，失败返回 `None`（不自旋）。
    ///
    /// # Safety
    ///
    /// 同 [`lock`](Self::lock)：调用者必须保证不从中断上下文争用。
    #[allow(dead_code)] // 非阻塞获取预留
    #[inline(never)] // 同 lock：保证入口 ra 为调用者返回地址
    pub unsafe fn try_lock(&self) -> Option<BareLockGuard<'_, T>> {
        let caller = dep::read_ra();
        // 失败不报重入——try_lock 语义允许拿不到
        if self.locked.swap(true, Ordering::Acquire) {
            return None;
        }
        self.holder_pc.store(caller, Ordering::Relaxed);

        Some(BareLockGuard {
            lock: self,
            _not_send: PhantomData,
        })
    }
}

impl<T: ?Sized> Deref for BareLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: 同一时刻只有一个 guard 存在，&self 对应唯一 &T。
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> DerefMut for BareLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: 同一时刻只有一个 guard 存在，&mut self 对应唯一 &mut T。
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for BareLockGuard<'_, T> {
    fn drop(&mut self) {
        // Release：保证之前写入在解锁时对其他核可见
        self.lock.locked.store(false, Ordering::Release);
        // 清除持有者调用点（AtomicUsize，与 data 同受锁互斥保护）
        self.lock.holder_pc.store(0, Ordering::Relaxed);
    }
}
