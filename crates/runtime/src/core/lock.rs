//! lock — 用户态互斥：同域跨任务共享一张表时的最小临界区原语。
//!
//! 与内核的 `SpinLock` 无关（那边保护内核结构、有锁序检查）；此处只保护**同域
//! 两线程**共享的用户数据，临界区极短（查表 / 改表）。
//!
//! 为什么是 `with` 而不是 `lock()`：**没有守卫可泄漏**，锁程 = 闭包程，调用点即
//! 临界区。不可重入——同一线程再进 `with` 会自旋自锁。
//!
//! 无中毒（poisoning）概念：内核 panic 即 abort，不存在带锁展开。

use core::cell::UnsafeCell;
use core::hint::spin_loop;
use core::sync::atomic::{AtomicBool, Ordering};

/// 自旋互斥。
pub struct Lock<T> {
    busy: AtomicBool,
    cell: UnsafeCell<T>,
}

// SAFETY: 互斥由 `busy` 保证——同一时刻至多一个线程拿到 `&mut T`；跨线程共享
// 因此还要求 `T: Send`。
unsafe impl<T: Send> Sync for Lock<T> {}

impl<T> Lock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            busy: AtomicBool::new(false),
            cell: UnsafeCell::new(value),
        }
    }

    /// 取锁 → 执行 → 放锁。
    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        while self.busy.swap(true, Ordering::Acquire) {
            spin_loop();
        }
        // SAFETY: 上面取得互斥，故此刻无人并发访问 `cell`；`f` 返回即放锁。
        let out = f(unsafe { &mut *self.cell.get() });
        self.busy.store(false, Ordering::Release);
        out
    }
}
