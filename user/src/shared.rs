//! 共享缓冲核心（pub(crate)）—— dock/ring 共用的"裸缓冲 + 自旋锁 + 单调弧"。

// 硬不变量：acquire/release 严格配对；持锁期间仅原子字段 + 槽位 memcpy；单调弧不回退。

use core::marker::PhantomData;
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

pub(crate) trait Offsets {
    const STATE: usize;
    const LOCK: usize;
    const READ: usize;
    const WRITE: usize;
    const ITEM_LEN: usize;
    const SLOTS: usize;
    const BUFFER: usize;
}

pub(crate) struct DockLayout;
pub(crate) struct RingLayout;

impl Offsets for DockLayout {
    const STATE: usize = ubi::dock::OFF_STATE;
    const LOCK: usize = ubi::dock::OFF_LOCK;
    const READ: usize = ubi::dock::OFF_READ;
    const WRITE: usize = ubi::dock::OFF_WRITE;
    const ITEM_LEN: usize = ubi::dock::OFF_ITEM_LEN;
    const SLOTS: usize = ubi::dock::OFF_SLOTS;
    const BUFFER: usize = ubi::dock::OFF_BUFFER;
}

impl Offsets for RingLayout {
    const STATE: usize = ubi::ring::OFF_STATE;
    const LOCK: usize = ubi::ring::OFF_LOCK;
    const READ: usize = ubi::ring::OFF_READ;
    const WRITE: usize = ubi::ring::OFF_WRITE;
    const ITEM_LEN: usize = ubi::ring::OFF_ITEM_LEN;
    const SLOTS: usize = ubi::ring::OFF_SLOTS;
    const BUFFER: usize = ubi::ring::OFF_BUFFER;
}

pub(crate) struct SharedBuf<L: Offsets> {
    base: *mut u8,
    _marker: PhantomData<L>,
}

impl<L: Offsets> Copy for SharedBuf<L> {}
impl<L: Offsets> Clone for SharedBuf<L> {
    fn clone(&self) -> Self {
        *self
    }
}

// SAFETY: 多任务多核共享同一物理帧；原子协议同步；base 页对齐、偏移按类型对齐。
unsafe impl<L: Offsets> Send for SharedBuf<L> {}
unsafe impl<L: Offsets> Sync for SharedBuf<L> {}

impl<L: Offsets> SharedBuf<L> {
    pub(crate) fn new(base: usize) -> Self {
        Self {
            base: base as *mut u8,
            _marker: PhantomData,
        }
    }

    #[inline]
    fn field<T>(&self, off: usize) -> *mut T {
        unsafe { self.base.add(off) as *mut T }
    }

    #[inline]
    pub(crate) fn state(&self) -> &AtomicU8 {
        unsafe { &*self.field(L::STATE) }
    }

    #[inline]
    fn lock(&self) -> &AtomicBool {
        unsafe { &*self.field(L::LOCK) }
    }

    #[inline]
    pub(crate) fn read(&self) -> &AtomicUsize {
        unsafe { &*self.field(L::READ) }
    }

    #[inline]
    pub(crate) fn write(&self) -> &AtomicUsize {
        unsafe { &*self.field(L::WRITE) }
    }

    #[inline]
    fn item_len(&self) -> usize {
        unsafe { *self.field::<usize>(L::ITEM_LEN) }
    }

    #[inline]
    fn slots(&self) -> usize {
        unsafe { *self.field::<usize>(L::SLOTS) }
    }

    #[inline]
    fn slot_ptr(&self, idx: usize) -> *mut u8 {
        let mask = self.slots() - 1;
        unsafe { self.base.add(L::BUFFER + (idx & mask) * self.item_len()) }
    }

    #[inline]
    pub(crate) fn acquire(&self) {
        while self.lock().swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }
    }

    #[inline]
    pub(crate) fn release(&self) {
        self.lock().store(false, Ordering::Release);
    }

    /// 0 = 成功；-2 = 满。
    pub(crate) fn try_push_locked(&self, msg: &[u8]) -> isize {
        debug_assert_eq!(msg.len(), self.item_len());
        let w = self.write().load(Ordering::Acquire);
        let r = self.read().load(Ordering::Acquire);
        if w.wrapping_sub(r) == self.slots() {
            return -2;
        }
        unsafe { ptr::copy_nonoverlapping(msg.as_ptr(), self.slot_ptr(w), msg.len()) };
        self.write().fetch_add(1, Ordering::Release);
        0
    }

    /// 0 = 成功；-2 = 空。
    pub(crate) fn try_pull_locked(&self, buf: &mut [u8]) -> isize {
        debug_assert_eq!(buf.len(), self.item_len());
        let w = self.write().load(Ordering::Acquire);
        let r = self.read().load(Ordering::Acquire);
        if w.wrapping_sub(r) == 0 {
            return -2;
        }
        unsafe { ptr::copy_nonoverlapping(self.slot_ptr(r), buf.as_mut_ptr(), buf.len()) };
        self.read().fetch_add(1, Ordering::Release);
        0
    }
}
