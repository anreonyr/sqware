// SpinLock — 中断安全自旋锁，多核基础
//
// lock() 返回 SpinLockGuard，通过 Deref/DerefMut 提供 &T/&mut T 访问。
// 获取锁时关闭 S-mode 全局中断（sstatus.SIE），guard 析构时释放锁并恢复中断。
// 这解决了"任务上下文持锁 → 中断抢占 → 中断处理路径争同一把锁"的死锁。
//
// 关中断逻辑委托给 TrapGuard（见 lock/trap.rs）；guard 携带 !Send 标记，
// 保证锁必须在获取它的同一 hart 上释放（多核安全前提）。
//
// lockdep：lock/try_lock 在入口捕获调用者返回地址（ra）写入 caller，
// 同 hart 重入由 depend::check（自旋前 contains）暴露（recursive acquisition）；
// 跨 hart 争用是真自旋（他核持有，check 通过）。

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::depend;
use super::trap::TrapGuard;

pub struct SpinLock<T: ?Sized> {
    locked: AtomicBool,
    /// 当前持有者调用点（返回地址；0 = 未持有）——死锁溯源用
    caller: AtomicUsize,
    /// 锁层级（参与 lockdep；None = exempt）。enforcement 仅 debug 生效。
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    level: Option<depend::Level>,
    data: UnsafeCell<T>,
}

// SAFETY: 同一时刻只有一个 SpinLockGuard 持有 &mut T，
// 且 guard 不实现 Send（锁必须在本 hart 上释放）。
// 无条件 Sync：内核需在跨上下文场景保护含裸指针的状态（如 MMIO、TrapFrame*）。
unsafe impl<T: ?Sized> Sync for SpinLock<T> {}

/// 锁守卫 — 持有锁期间通过 Deref/DerefMut 访问受保护数据。
///
/// 析构时自动释放锁并恢复中断使能状态。
pub struct SpinLockGuard<'a, T: ?Sized> {
    lock: &'a SpinLock<T>,
    // *const () 既不 Send 也不 Sync：强制 guard 在本 hart 上释放
    _not_send: PhantomData<*const ()>,
    // 持有期间关中断，其 Drop 在 guard Drop 之后执行以恢复 SIE
    _trap: TrapGuard,
}

impl<T> SpinLock<T> {
    pub const fn new(val: T) -> Self {
        SpinLock {
            locked: AtomicBool::new(false),
            caller: AtomicUsize::new(0),
            level: None,
            data: UnsafeCell::new(val),
        }
    }

    /// 带锁层级的构造（参与 lockdep；new() 默认 None = exempt）。
    pub const fn new_level(level: depend::Level, val: T) -> Self {
        SpinLock {
            locked: AtomicBool::new(false),
            caller: AtomicUsize::new(0),
            level: Some(level),
            data: UnsafeCell::new(val),
        }
    }
}

impl<T: ?Sized> SpinLock<T> {
    /// 当前持有者调用点（返回地址；0 = 未持有）——死锁溯源诊断用。
    ///
    /// 预留 API：panic/死锁场景下经 SBI 无锁输出，供定位"谁锁着不释放"。
    #[allow(dead_code)]
    pub fn caller(&self) -> usize {
        self.caller.load(Ordering::Relaxed)
    }

    /// 获取锁，返回守卫。
    ///
    /// 获取前关闭 S-mode 全局中断（sstatus.SIE=0），
    /// 防止同 CPU 中断上下文重入导致死锁。
    /// 守卫析构时释放锁并恢复 SIE。
    ///
    /// 同 hart 重入由 depend::check（自旋前 contains）暴露并 panic；
    /// 跨 hart 争用是真自旋（他核持有，check 通过后自旋等待）。
    #[inline(never)] // 保证入口读到的 ra 是调用者返回地址（内联会破坏）
    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        // 入口第一件事：读调用点（任何函数调用会覆盖 ra）。
        let caller = crate::lock::depend_enter!(self);
        // SAFETY: 处于 S-mode；关中断防止本 hart 中断重入。
        let trap = unsafe { TrapGuard::save() };
        // lockdep：自旋前层级校验（关中断后——held set 仅 SIE 关时可写）。
        crate::lock::depend_check!(self, caller);

        // Acquire：获取成功后看到前持有者的所有写入。跨 hart 争用时真自旋。
        while self.locked.swap(true, Ordering::Acquire) {
            // 自旋核收不到 trap：就地留意停机报警并卧倒。
            crate::runtime::diagnose::halt::hush();
            core::hint::spin_loop();
        }
        self.caller.store(caller, Ordering::Relaxed);
        // lockdep：取到后记入持有集（顺带记本次调用点）。
        crate::lock::depend_acquire!(self, caller);

        SpinLockGuard {
            lock: self,
            _not_send: PhantomData,
            _trap: trap,
        }
    }

    /// 尝试获取锁，成功返回守卫，失败返回 `None`（不自旋）。
    ///
    /// 失败不报重入——`try_lock` 语义允许拿不到；成功时同样记录持有者调用点。
    /// 非阻塞路径无死锁可能（拿不到即弃）——不做 check（顺序校验只对阻塞
    /// lock() 生效，那是 ABBA 发生的边界）。
    #[allow(dead_code)] // 非阻塞获取预留
    #[inline(never)] // 同 lock：保证入口 ra 为调用者返回地址
    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        let caller: usize;
        // SAFETY: 读 ra 无副作用；asm 未声明 ra 视为 clobber，编译器不假设它保持。
        unsafe { core::arch::asm!("mv {0}, ra", out(reg) caller) };
        // SAFETY: 处于 S-mode；关中断防止本 hart 中断重入。
        let trap = unsafe { TrapGuard::save() };

        // 获取失败时 trap 析构自动恢复中断
        if self.locked.swap(true, Ordering::Acquire) {
            return None;
        }
        self.caller.store(caller, Ordering::Relaxed);
        // lockdep：取到后记入持有集。
        crate::lock::depend_acquire!(self, caller);

        Some(SpinLockGuard {
            lock: self,
            _not_send: PhantomData,
            _trap: trap,
        })
    }
}

impl<T: ?Sized> Deref for SpinLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: SpinLock 保证同一时刻只有一个 guard 存在，
        // 且 guard 的 &self 对应唯一的 &T。
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> DerefMut for SpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: SpinLock 保证同一时刻只有一个 guard 存在，
        // 且 guard 的 &mut self 对应唯一的 &mut T。
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for SpinLockGuard<'_, T> {
    fn drop(&mut self) {
        // lockdep：释放时移除持有集条目。
        crate::lock::depend_release!(self.lock);

        // Release：保证之前写入在解锁时对其他核可见
        self.lock.locked.store(false, Ordering::Release);
        // 清除持有者调用点（AtomicUsize，与 data 同受锁互斥保护）
        self.lock.caller.store(0, Ordering::Relaxed);
        // _trap 字段随后析构，恢复 SIE
    }
}
