// SpinLock — 中断安全自旋锁，多核基础
//
// lock() 返回 SpinLockGuard，通过 Deref/DerefMut 提供 &T/&mut T 访问。
// 获取锁时关闭 S-mode 全局中断（sstatus.SIE），guard 析构时释放锁并恢复中断。
// 这解决了"任务上下文持锁 → 中断抢占 → 中断处理路径争同一把锁"的死锁。
//
// 关中断逻辑委托给 TrapGuard（见 lock/trap.rs）；guard 携带 !Send 标记，
// 保证锁必须在获取它的同一 hart 上释放（多核安全前提）。
//
// lockdep 最小版：lock/try_lock 在入口捕获调用者返回地址（ra）写入 holder_pc，
// 单 hart 下 swap 发现锁已被持有必然是同 hart 重入（关中断后无抢占）——
// 在必然死循环前报告持有者/本次调用点并 panic（depend::report）。

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::machine;

use super::depend;
use super::trap::TrapGuard;

/// 自旋等待打点节流（ticks；timebase 10MHz 下 1ms）。自旋核经此报岗「有进展」，
/// 使 watch 的 B 判据（BEAT 过期 → "stalled"）不再把等锁自旋误判为失速；须远
/// 小于 boot 注入的 liveness_timeout（200ms），1ms 留足余量。SpinLock 与
/// RelLock 的自旋路径共用（见 reentrant.rs）。
pub(crate) const SPIN_PULSE_TICKS: u64 = 10_000;

pub struct SpinLock<T: ?Sized> {
    locked: AtomicBool,
    /// 持有者 hart_id + 1（0 = 空闲）——区分同 hart 重入与跨 hart 争用。
    /// 语义对齐 RelLock：同 hart 再次获取是锁序违规（panic），跨 hart 是真争用（自旋）。
    owner: AtomicUsize,
    /// 当前持有者调用点（返回地址；0 = 未持有）——递归获取检测与死锁溯源用
    holder_pc: AtomicUsize,
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
            owner: AtomicUsize::new(0),
            holder_pc: AtomicUsize::new(0),
            level: None,
            data: UnsafeCell::new(val),
        }
    }

    /// 带锁层级的构造（参与 lockdep；new() 默认 None = exempt）。
    pub const fn new_level(level: depend::Level, val: T) -> Self {
        SpinLock {
            locked: AtomicBool::new(false),
            owner: AtomicUsize::new(0),
            holder_pc: AtomicUsize::new(0),
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
    pub fn holder_pc(&self) -> usize {
        self.holder_pc.load(Ordering::Relaxed)
    }

    /// 获取锁，返回守卫。
    ///
    /// 获取前关闭 S-mode 全局中断（sstatus.SIE=0），
    /// 防止同 CPU 中断上下文重入导致死锁。
    /// 守卫析构时释放锁并恢复 SIE。
    ///
    /// 单 hart 下锁已被持有必然是同 hart 重入（关中断后无抢占）——
    /// 在必然死循环前报告持有者与本次调用点（lockdep 最小版）。
    #[inline(never)] // 保证入口读到的 ra 是调用者返回地址（内联会破坏）
    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        // 入口第一件事：捕获调用者返回地址（任何函数调用都会覆盖 ra）
        let caller = depend::ra();
        // SAFETY: 处于 S-mode；关中断防止本 hart 中断重入。
        let trap = unsafe { TrapGuard::save() };
        // SAFETY: 读 tp（入口 `_start` 写入的 hartid）无副作用。
        let me = machine::hart_id() + 1;

        // lockdep：自旋前校验（抓 ABBA——死锁发生在自旋后，须先暴露）。
        #[cfg(debug_assertions)]
        if let Some(lv) = self.level {
            depend::check(self as *const Self as *const () as usize, lv, caller);
        }

        // 自旋打点节流本地时钟（见 SPIN_PULSE_TICKS；仅自旋分支使用）。
        let mut last_pulse = 0u64;

        // Acquire：获取成功后看到前持有者的所有写入。跨 hart 争用时真自旋；
        // 同 hart 再次获取（关中断后无抢占，必然是同 hart 重入）是锁序违规——panic。
        while self.locked.swap(true, Ordering::Acquire) {
            if self.owner.load(Ordering::Relaxed) == me {
                let holder = self.holder_pc.load(Ordering::Relaxed);
                depend::report(
                    "spinlock",
                    "recursive acquisition",
                    self as *const Self as *const () as usize,
                    holder,
                    caller,
                );
            } else {
                // 跨核争用：本地看门狗盯住这把被抢的锁（持方 + 起始时刻）。
                crate::runtime::diagnose::watch::stake(
                    self as *const Self as *const () as usize,
                    self.owner.load(Ordering::Relaxed),
                    self.holder_pc.load(Ordering::Relaxed),
                );
            }
            // 自旋等待者也是「活着」的形态：看警（ALARM 已拉响且他核报警 → 就地
            // 卧倒——修补自旋核不收 trap、hush 钩子覆盖不到的停机漏洞），并按节流
            // 报岗（自旋即有进展，B 判据不误伤；锁相持交 A 判据在语义确凿时报告）。
            crate::runtime::diagnose::halt::hush();
            let t = crate::runtime::chrono::clock::now().as_ticks();
            if t.wrapping_sub(last_pulse) >= SPIN_PULSE_TICKS {
                crate::runtime::diagnose::watch::pulse();
                last_pulse = t;
            }
            core::hint::spin_loop();
        }
        self.owner.store(me, Ordering::Relaxed);
        self.holder_pc.store(caller, Ordering::Relaxed);

        // lockdep：取到后记入持有集。
        #[cfg(debug_assertions)]
        if let Some(lv) = self.level {
            depend::acquire(self as *const Self as *const () as usize, lv);
        }

        SpinLockGuard {
            lock: self,
            _not_send: PhantomData,
            _trap: trap,
        }
    }

    /// 尝试获取锁，成功返回守卫，失败返回 `None`（不自旋）。
    ///
    /// 失败不报重入——`try_lock` 语义允许拿不到；成功时同样记录持有者调用点。
    #[allow(dead_code)] // 非阻塞获取预留
    #[inline(never)] // 同 lock：保证入口 ra 为调用者返回地址
    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        let caller = depend::ra();
        // SAFETY: 处于 S-mode；关中断防止本 hart 中断重入。
        let trap = unsafe { TrapGuard::save() };

        // lockdep：try_lock 非阻塞、拿不到即弃，无死锁可能——不做顺序校验
        // （但仍记入持有集）。顺序校验只对阻塞 lock() 生效，那是 ABBA 发生的边界。

        // 获取失败时 trap 析构自动恢复中断
        if self.locked.swap(true, Ordering::Acquire) {
            return None;
        }
        // SAFETY: 读 tp（入口 `_start` 写入的 hartid）无副作用。
        let me = machine::hart_id() + 1;
        self.owner.store(me, Ordering::Relaxed);
        self.holder_pc.store(caller, Ordering::Relaxed);

        // lockdep：取到后记入持有集。
        #[cfg(debug_assertions)]
        if let Some(lv) = self.level {
            depend::acquire(self as *const Self as *const () as usize, lv);
        }

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
        // 本地看门狗撤哨（若正盯这把锁；未盯即免，热路径廉价）。
        crate::runtime::diagnose::watch::unstake(
            self.lock as *const SpinLock<T> as *const () as usize,
        );
        // lockdep：释放时移除持有集条目。
        #[cfg(debug_assertions)]
        if let Some(lv) = self.lock.level {
            depend::release(self.lock as *const _ as *const () as usize, lv);
        }

        // Release：保证之前写入在解锁时对其他核可见
        self.lock.locked.store(false, Ordering::Release);
        // 清除持有者调用点与持有者 hart（AtomicUsize，与 data 同受锁互斥保护）
        self.lock.holder_pc.store(0, Ordering::Relaxed);
        self.lock.owner.store(0, Ordering::Relaxed);
        // _trap 字段随后析构，恢复 SIE
    }
}
