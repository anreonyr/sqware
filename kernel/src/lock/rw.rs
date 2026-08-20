// RwLock — 中断安全读写锁，多读单写
//
// 用单个 AtomicUsize 编码状态：最高位 WRITER_BIT 表示写者持有，
// 低位记录活跃读者数。多个读者可并发持有，写者独占。
// 读写获取都关中断（复用 TrapGuard），可从中断上下文安全获取。
//
// 写者优先策略：一旦写者置位 WRITER_BIT，新读者无法再获取，防止写者饿死。
// guard 携带 !Send 标记，保证在本 hart 释放。

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicUsize, Ordering};

use super::depend;
use super::trap::TrapGuard;

// 最高位：写者持有标志
const WRITER_BIT: usize = 1 << (usize::BITS - 1);
// 低位掩码：读者计数
#[allow(dead_code)] // 写路径当前无调用方（RwLock 仅 hub 查询用读路径），保留
const READER_MASK: usize = !WRITER_BIT;

pub struct RwLock<T: ?Sized> {
    // WRITER_BIT | reader_count
    state: AtomicUsize,
    /// 写锁持有者调用点（返回地址；0 = 无写者）——写重入/升级/降级检测与溯源用
    holder_pc: AtomicUsize,
    data: UnsafeCell<T>,
}

// SAFETY: 读者需要 T: Sync（多个 &T 跨 hart），写者需要 T: Send（&mut T 转移）。
// guard !Send，锁在本 hart 释放。
unsafe impl<T: ?Sized + Send + Sync> Sync for RwLock<T> {}

/// 读守卫 — 持有期间通过 Deref 访问 &T，析构时释放读锁。
pub struct RwLockReadGuard<'a, T: ?Sized> {
    lock: &'a RwLock<T>,
    _not_send: PhantomData<*const ()>,
    _trap: TrapGuard,
}

/// 写守卫 — 持有期间通过 Deref/DerefMut 访问 &mut T，析构时释放写锁。
#[allow(dead_code)] // 写路径当前无调用方（RwLock 仅 hub 查询用读路径），保留
pub struct RwLockWriteGuard<'a, T: ?Sized> {
    lock: &'a RwLock<T>,
    _not_send: PhantomData<*const ()>,
    _trap: TrapGuard,
}

impl<T> RwLock<T> {
    pub const fn new(val: T) -> Self {
        RwLock {
            state: AtomicUsize::new(0),
            holder_pc: AtomicUsize::new(0),
            data: UnsafeCell::new(val),
        }
    }
}

impl<T: ?Sized> RwLock<T> {
    /// 获取读锁，返回读守卫。多个读者可并发持有。
    ///
    /// 若已有写者持有或等待，则自旋等待。获取期间关中断。
    ///
    /// 单 hart 下读重入合法（计数递增）；但持写锁再读（降级）是死锁——
    /// 写者即本执行流，等待写者离开永不发生。
    #[inline(never)] // 保证入口读到的 ra 是调用者返回地址（内联会破坏）
    pub fn read(&self) -> RwLockReadGuard<'_, T> {
        // 入口第一件事：捕获调用者返回地址（任何函数调用都会覆盖 ra）
        let caller = depend::ra();
        // SAFETY: 处于 S-mode；关中断防止本 hart 中断重入。
        let trap = unsafe { TrapGuard::save() };

        // Acquire：读者进入临界区前看到写者的所有写入
        let s = self.state.fetch_add(1, Ordering::Acquire);
        if s & WRITER_BIT != 0 {
            // 有写者：单 hart 下写者必是本执行流（持写锁再读）→ 撤销计数后报告
            self.state.fetch_sub(1, Ordering::Release);
            let holder = self.holder_pc.load(Ordering::Relaxed);
            depend::report(
                "rwlock",
                "write→read downgrade deadlock",
                self as *const Self as *const () as usize,
                holder,
                caller,
            );
        }

        RwLockReadGuard {
            lock: self,
            _not_send: PhantomData,
            _trap: trap,
        }
    }

    /// 获取写锁，返回写守卫。写者独占。
    ///
    /// 先置 WRITER_BIT 阻塞新读者，再等待现存读者归零。获取期间关中断。
    ///
    /// 单 hart 下两个死锁形态在此捕获：写重入（WRITER_BIT 已置位即自己）、
    /// 读→写升级（读者计数非 0 必是自己的读锁）。
    #[inline(never)] // 同 read：保证入口 ra 为调用者返回地址
    #[allow(dead_code)] // 写路径当前无调用方（RwLock 仅 hub 查询用读路径），保留
    pub fn write(&self) -> RwLockWriteGuard<'_, T> {
        // 入口第一件事：捕获调用者返回地址
        let caller = depend::ra();
        // SAFETY: 处于 S-mode；关中断防止本 hart 中断重入。
        let trap = unsafe { TrapGuard::save() };

        // 抢占 WRITER_BIT：从"无写者"状态置位
        loop {
            let s = self.state.load(Ordering::Relaxed);
            if s & WRITER_BIT != 0 {
                // 已有写者：单 hart 下写者必是本执行流 → 写重入，报告后 panic
                let holder = self.holder_pc.load(Ordering::Relaxed);
                depend::report(
                    "rwlock",
                    "recursive write acquisition",
                    self as *const Self as *const () as usize,
                    holder,
                    caller,
                );
            }
            if self
                .state
                .compare_exchange(s, s | WRITER_BIT, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                break;
            }
            // cas 失败：单 hart 下 WRITER_BIT 已检查过，理论不可达——保守自旋防多核
            core::hint::spin_loop();
        }

        // 等待现存读者全部离开：单 hart 下读者计数非 0 必是本执行流的读锁（升级死锁）
        if self.state.load(Ordering::Acquire) & READER_MASK != 0 {
            let holder = self.holder_pc.load(Ordering::Relaxed);
            depend::report(
                "rwlock",
                "read→write upgrade deadlock",
                self as *const Self as *const () as usize,
                holder,
                caller,
            );
        }
        self.holder_pc.store(caller, Ordering::Relaxed);

        RwLockWriteGuard {
            lock: self,
            _not_send: PhantomData,
            _trap: trap,
        }
    }
}

impl<T: ?Sized> Deref for RwLockReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: 持有读锁期间无写者，可安全共享 &T。
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for RwLockReadGuard<'_, T> {
    fn drop(&mut self) {
        // Release：读者退出前的读取对后续写者可见
        self.lock.state.fetch_sub(1, Ordering::Release);
    }
}

impl<T: ?Sized> Deref for RwLockWriteGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: 写锁独占，无其他访问者。
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> DerefMut for RwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: 写锁独占，&mut self 对应唯一 &mut T。
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for RwLockWriteGuard<'_, T> {
    fn drop(&mut self) {
        // 清除写持有者调用点
        self.lock.holder_pc.store(0, Ordering::Relaxed);
        // Release：清除 WRITER_BIT（读者计数此刻为 0），写入对后续获取者可见
        self.lock.state.store(0, Ordering::Release);
    }
}
