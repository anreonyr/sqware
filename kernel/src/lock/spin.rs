use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::depend;
use super::trap::TrapGuard;

pub struct SpinLock<T: ?Sized> {
    locked: AtomicBool,
    caller: AtomicUsize,
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    level: Option<depend::Level>,
    data: UnsafeCell<T>,
}

// SAFETY: 单 guard 持 &mut T；guard !Send（锁在本 hart 释放）
unsafe impl<T: ?Sized> Sync for SpinLock<T> {}

pub struct SpinLockGuard<'a, T: ?Sized> {
    lock: &'a SpinLock<T>,
    _not_send: PhantomData<*const ()>,
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
    #[allow(dead_code)]
    pub fn caller(&self) -> usize {
        self.caller.load(Ordering::Relaxed)
    }

    #[inline(never)]
    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        let caller = crate::lock::depend_enter!(self);
        // SAFETY: 处于 S-mode；关中断防止本 hart 中断重入
        let trap = unsafe { TrapGuard::save() };
        crate::lock::depend_check!(self, caller);

        // **自旋太久就报一句**（**打印那一路是免锁的**：`console::_write` 只往栈上那格缓冲里
        // 格式化、再 SBI 直写，故这一行在"整机不出声"时也出得来）。
        //
        // **为什么需要它**：抱锁的那一位若永远不放手，等的人就在这儿 `spin_loop` 转下去，
        // 而**这一刻本 hart 的 IRQs 是关着的**（`TrapGuard::save`）⇒ 定时器也叫不动它；
        // `hush()` 只在本 hart 撞上"有别的 hart 认领了报警"时才自停。于是整机**一条读数都没有**。
        // 量到的现象正是这一形：accept 景偶发"整机跑完不出场"——控制台在装配中途整段停住、
        // 连 `idle 10000ms with walkers alive` 那条看门狗都不响、也没有 verdict。
        // 报的是**等级 ＋ 地址 ＋ 抱锁那位的 `ra` ＋ 等的人 `ra`**（`ra` 落到哪个函数，
        // 用 `cargo nm` 对一下就知道）——一枚锁每次入 `lock` 最多一行。
        const STUCK_SPINS: usize = 1 << 26;
        let mut spins: usize = 0;
        let mut told = false;
        while self.locked.swap(true, Ordering::Acquire) {
            crate::runtime::diagnose::halt::hush();
            core::hint::spin_loop();
            spins += 1;
            if !told && spins >= STUCK_SPINS {
                told = true;
                crate::putln!(
                    "lock: stuck addr={:#x} level={:?} holder=ra{:#x} waiter=ra{:#x}",
                    self as *const Self as *const () as usize,
                    self.level,
                    self.caller.load(Ordering::Relaxed),
                    caller,
                );
            }
        }
        self.caller.store(caller, Ordering::Relaxed);
        crate::lock::depend_acquire!(self, caller);

        SpinLockGuard {
            lock: self,
            _not_send: PhantomData,
            _trap: trap,
        }
    }

    #[allow(dead_code)]
    #[inline(never)]
    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        let caller: usize;
        // SAFETY: 读 ra（asm 未声明视为 clobber）
        unsafe { core::arch::asm!("mv {0}, ra", out(reg) caller) };
        // SAFETY: 处于 S-mode；关中断防止本 hart 中断重入
        let trap = unsafe { TrapGuard::save() };

        if self.locked.swap(true, Ordering::Acquire) {
            return None;
        }
        self.caller.store(caller, Ordering::Relaxed);
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
        // SAFETY: 单 guard 持锁期间无其他持有者
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> DerefMut for SpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: 单 guard 持锁期间无其他持有者
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for SpinLockGuard<'_, T> {
    fn drop(&mut self) {
        crate::lock::depend_release!(self.lock);
        self.lock.locked.store(false, Ordering::Release);
        self.lock.caller.store(0, Ordering::Relaxed);
    }
}
