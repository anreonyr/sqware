// RelLock — 中断安全可重入锁
//
// 同一 hart 可重复获取而不死锁：首次获取记录持有者 hart id 并置重入计数为 1，
// 再次获取仅递增计数；释放时递减，归零时才真正释放。不同 hart 之间仍互斥。
//
// 用途：临界区内可能重入获取同一把锁的场景（如持锁期间触发缺页、
// 缺页处理器再次获取同一把锁）。获取期间关中断（复用 TrapGuard）。
//
// owner 存储 "hart_id + 1"：0 表示空闲（hart 0 是合法 id，故需 +1 偏移）。
// guard 携带 !Send 标记，保证在本 hart 释放。

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::machine;

use super::depend;
use super::trap::TrapGuard;

#[derive(Debug)]
pub struct RelLock<T: ?Sized> {
    // 持有者 hart_id + 1；0 = 空闲
    owner: AtomicUsize,
    // 重入计数：>0 表示被持有
    count: UnsafeCell<usize>,
    /// 最外层持有者调用点（返回地址；0 = 空闲）——死锁溯源用。
    /// 重入是 RelLock 的合法语义，不检测；仅记录首次获取的调用点。
    caller: AtomicUsize,
    /// 锁层级（参与 lockdep；None = exempt）。enforcement 仅 debug 生效。
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    level: Option<depend::Level>,
    data: UnsafeCell<T>,
}

// SAFETY: 同一时刻只有一个 hart 持有锁；count 仅在持锁时访问。
// guard !Send，锁在本 hart 释放。
unsafe impl<T: ?Sized + Send> Sync for RelLock<T> {}

/// 锁守卫 — 持有期间通过 Deref/DerefMut 访问受保护数据。
///
/// 析构时递减重入计数，归零时释放锁并恢复中断。
pub struct RelLockGuard<'a, T: ?Sized> {
    lock: &'a RelLock<T>,
    // *const () 既不 Send 也不 Sync：强制 guard 在本 hart 上释放
    _not_send: PhantomData<*const ()>,
    // 关中断守卫，Drop 时恢复 SIE（在释放锁之后）
    _trap: TrapGuard,
}

impl<T> RelLock<T> {
    pub const fn new(val: T) -> Self {
        RelLock {
            owner: AtomicUsize::new(0),
            count: UnsafeCell::new(0),
            caller: AtomicUsize::new(0),
            level: None,
            data: UnsafeCell::new(val),
        }
    }

    /// 带锁层级的构造（参与 lockdep；new() 默认 None = exempt）。
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    pub const fn new_level(level: depend::Level, val: T) -> Self {
        RelLock {
            owner: AtomicUsize::new(0),
            count: UnsafeCell::new(0),
            caller: AtomicUsize::new(0),
            level: Some(level),
            data: UnsafeCell::new(val),
        }
    }
}

impl<T: ?Sized> RelLock<T> {
    /// 无锁只读访问指针（诊断/打印路径专用）：调用方必须保证此刻无并发写
    /// （内核空间装配后只读；故障现场其他核已停）。正常路径勿用——会绕过锁。
    pub(crate) fn read_unlocked(&self) -> *const T {
        self.data.get()
    }

    /// Acquire the lock, returning a guard. Reentrant on the same hart.
    ///
    /// On first acquisition, spins until the lock is free. If already held by this
    /// hart, increments the reentrancy count. Interrupts are disabled during
    /// acquisition (via `TrapGuard`) to prevent interrupt-induced deadlock on the
    /// same hart.
    ///
    /// Note: during early boot (before `sstatus::set(SIE)`), the `TrapGuard` CSR
    /// save/restore is redundant since interrupts are not yet enabled. This is
    /// accepted for simplicity — a `lock_noirq()` fast path could be added if
    /// profiling shows it matters.
    #[inline(never)] // 保证入口读到的 ra 是调用者返回地址（内联会破坏）
    pub fn lock(&self) -> RelLockGuard<'_, T> {
        // 入口第一件事：捕获调用者返回地址（任何函数调用都会覆盖 ra）
        let caller: usize;
        // SAFETY: 读 ra 无副作用；asm 未声明 ra 视为 clobber，编译器不假设它保持。
        unsafe { core::arch::asm!("mv {}, ra", out(reg) caller) };
        // SAFETY: 处于 S-mode；关中断防止本 hart 中断重入。
        let trap = unsafe { TrapGuard::save() };
        // SAFETY: 读 tp 指向的 PerHart.id（经 hart_id()）无副作用；多 hart 时各核各异。
        let me = machine::hart_id() + 1;

        // lockdep：非重入（本核尚未持有）才做取前校验；同锁重入合法，跳过。
        #[cfg(debug_assertions)]
        if self.owner.load(Ordering::Relaxed) != me {
            depend::check(
                self as *const Self as *const () as usize,
                self.level,
                caller,
            );
        }

        loop {
            // Acquire：获取成功后看到前持有者的所有写入
            match self
                .owner
                .compare_exchange(0, me, Ordering::Acquire, Ordering::Relaxed)
            {
                Ok(_) => {
                    // 首次获取：重入计数置 1，记录最外层调用点
                    // SAFETY: 刚获得独占所有权，count 仅本 hart 访问。
                    unsafe { *self.count.get() = 1 };
                    self.caller.store(caller, Ordering::Relaxed);
                    // lockdep：首获记入持有集（顺带记获取点；exempt 记 None——
                    // 与 guard Drop 的 release 平衡，见 depend::acquire）。
                    #[cfg(debug_assertions)]
                    depend::acquire(
                        self as *const Self as *const () as usize,
                        self.level,
                        caller,
                    );
                    break;
                }
                Err(cur) if cur == me => {
                    // 本 hart 已持有：递增重入计数（合法重入，不更新 caller）
                    // SAFETY: 本 hart 持锁，count 仅本 hart 访问。
                    unsafe { *self.count.get() += 1 };
                    break;
                }
                Err(_) => {
                    // 其他 hart 持有：自旋等待（单 hart 下不可达，多核协议保留）；
                    // 自旋核收不到 trap，就地留意停机报警并卧倒。
                    crate::runtime::diagnose::halt::hush();
                    core::hint::spin_loop();
                }
            }
        }

        RelLockGuard {
            lock: self,
            _not_send: PhantomData,
            _trap: trap,
        }
    }
}

impl<T: ?Sized> Deref for RelLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: 本 hart 持有锁，访问受保护数据安全。
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> DerefMut for RelLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: 本 hart 独占持有锁，&mut self 对应唯一 &mut T。
        // 注意：重入期间获取多个 &mut 由借用检查在各 guard 生命周期内约束。
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for RelLockGuard<'_, T> {
    fn drop(&mut self) {
        // SAFETY: 本 hart 持锁，count 仅本 hart 访问。
        let c = unsafe {
            let p = self.lock.count.get();
            *p -= 1;
            *p
        };
        if c == 0 {
            // lockdep：外层释放时移除持有集条目。
            #[cfg(debug_assertions)]
            depend::release(self.lock as *const _ as *const () as usize);
            // 重入计数归零：释放锁，清除最外层调用点。Release 保证写入对后续获取者可见。
            self.lock.caller.store(0, Ordering::Relaxed);
            self.lock.owner.store(0, Ordering::Release);
        }
        // _trap 随后析构，恢复 SIE
    }
}
