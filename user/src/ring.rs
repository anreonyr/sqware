//! 用户侧 ring — 一对一共享内存邮路。
//!
//! 数据面 = 共享物理帧上的环形缓冲（布局契约 `ubi::ring`，与内核 mail::ring
//! 同源）。push/pull 在**本地视图基址**上原子操作，与 dock 同构但**语义更简**：
//! open 即两端固定（Producer / Consumer 各持一端），无 pier/quay 多对一计数、
//! 无 Hang/Gone 中间态——任一端 close（或对端离场）→ Dead。
//!
//! 阻塞语义同 dock：条件循环 + 调度域 wait/wake（ring 键带 RING_KEY_TAG）；
//! 断开感知 = 负码 Dead（-1）/ Busy（-2）。

use core::ptr;
use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use erra::ResultExt;
use ubi::ring::{self, RING_KEY_TAG};
use ubi::{UError, UResult};

use crate::env;

/// ring 状态编码（与内核 `mail::ring::RingState` 同值）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RingState {
    Live,
    Dead,
}

impl RingState {
    fn from_code(code: u8) -> RingState {
        match code {
            ring::state::LIVE => RingState::Live,
            _ => RingState::Dead,
        }
    }

    /// 是否仍可消费。
    pub const fn pullable(self) -> bool {
        matches!(self, RingState::Live)
    }
}

/// 共享区原子视图（本地映射基址 + 固定偏移；与 ubi::ring 布局配对）。
///
/// 所有字段都经原子访问（多任务多核共享同一物理帧）。
#[derive(Clone, Copy)]
pub(crate) struct Shared {
    base: *mut u8,
}

// SAFETY: Shared 持有的 base 是用户空间已映射 VA；原子访问无 Rust 别名，多线程
// 经原子协议同步——Send/Sync 安全（对齐保证：base 页对齐、偏移按类型对齐）。
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

impl Shared {
    /// 视图包装：`base` 为 ring 共享区本地映射基址（页对齐）。
    pub(crate) fn new(base: usize) -> Self {
        Self {
            base: base as *mut u8,
        }
    }

    #[inline]
    fn field<T>(&self, off: usize) -> *mut T {
        // SAFETY: 调用方保证 base 有效、偏移在共享区内、对齐满足 T。
        unsafe { self.base.add(off) as *mut T }
    }

    #[inline]
    fn state(&self) -> &AtomicU8 {
        // SAFETY: 布局契约。
        unsafe { &*self.field(ring::OFF_STATE) }
    }
    #[inline]
    fn lock(&self) -> &core::sync::atomic::AtomicBool {
        // SAFETY: 布局契约。
        unsafe { &*self.field(ring::OFF_LOCK) }
    }
    #[inline]
    fn read(&self) -> &AtomicUsize {
        // SAFETY: 布局契约。
        unsafe { &*self.field(ring::OFF_READ) }
    }
    #[inline]
    fn write(&self) -> &AtomicUsize {
        // SAFETY: 布局契约。
        unsafe { &*self.field(ring::OFF_WRITE) }
    }
    #[inline]
    fn item_len(&self) -> usize {
        // SAFETY: 只读字段 open 定型后不变；offset 在共享区内。
        unsafe { *self.field::<usize>(ring::OFF_ITEM_LEN) }
    }
    #[inline]
    fn slots(&self) -> usize {
        // SAFETY: 只读字段 open 定型后不变。
        unsafe { *self.field::<usize>(ring::OFF_SLOTS) }
    }

    /// 槽 i 的数据指针（2 的幂掩码环绕定位）。
    #[inline]
    fn slot_ptr(&self, idx: usize) -> *mut u8 {
        let mask = self.slots() - 1;
        // SAFETY: 偏移在共享区内（buffer 起点 + 槽位）；调用方按槽长访问。
        unsafe {
            self.base
                .add(ring::OFF_BUFFER + (idx & mask) * self.item_len())
        }
    }

    /// 取锁（自旋：swap 1 → 持锁）。
    #[inline]
    fn acquire(&self) {
        while self.lock().swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }
    }

    /// 放锁。
    #[inline]
    fn release(&self) {
        self.lock().store(false, Ordering::Release);
    }

    /// 投递尝试（锁内）：满 → Busy；Dead → 负码。返回错误码（0 = 成功）。
    fn try_push_locked(&self, msg: &[u8]) -> isize {
        debug_assert_eq!(msg.len(), self.item_len(), "ring push message size");
        let w = self.write().load(Ordering::Acquire);
        let r = self.read().load(Ordering::Acquire);
        if w.wrapping_sub(r) == self.slots() {
            return ring::err::BUSY;
        }
        // SAFETY: slot_ptr 指向映射内槽位；len == item_len 恒定。
        unsafe {
            ptr::copy_nonoverlapping(msg.as_ptr(), self.slot_ptr(w), msg.len());
        }
        self.write().fetch_add(1, Ordering::Release);
        0
    }

    /// 收取尝试（锁内）：空 → Busy；断开 → 负码。
    fn try_pull_locked(&self, buf: &mut [u8]) -> isize {
        debug_assert_eq!(buf.len(), self.item_len(), "ring pull buffer size");
        let w = self.write().load(Ordering::Acquire);
        let r = self.read().load(Ordering::Acquire);
        if w.wrapping_sub(r) == 0 {
            return ring::err::BUSY;
        }
        // SAFETY: slot_ptr 指向映射内槽位；len == item_len 恒定。
        unsafe {
            ptr::copy_nonoverlapping(self.slot_ptr(r), buf.as_mut_ptr(), buf.len());
        }
        self.read().fetch_add(1, Ordering::Release);
        0
    }
}

/// 生产端句柄（open 方唯一；无 clone——一对一）。
pub struct Producer {
    id: usize,
    shared: Shared,
}

/// 消费端句柄（open 方唯一）。
pub struct Consumer {
    id: usize,
    shared: Shared,
}

impl Producer {
    /// ring 全局 id（close / join 用）。
    pub fn id(&self) -> usize {
        self.id
    }
}

impl Consumer {
    /// ring 全局 id（close / join 用）。
    pub fn id(&self) -> usize {
        self.id
    }
}

/// 建 ring：返回 (Producer, Consumer) 两端同源（同视图基址）。id = ring 全局 id。
pub fn open(item_len: usize, slots: usize) -> UResult<(Producer, Consumer)> {
    let (id, view) = env::ring_open(item_len, slots)?;
    let shared = Shared::new(view);
    Ok((Producer { id, shared }, Consumer { id, shared }))
}

impl Producer {
    /// 投递尝试（非阻塞）：成功 Ok；Err -2 = 满（Busy，wait 后重试）、-1 = Dead。
    pub fn try_push(&self, msg: &[u8]) -> UResult<()> {
        // 状态预检（锁外读，防持锁自旋过久；锁内再校验）
        let st = RingState::from_code(self.shared.state().load(Ordering::Acquire));
        if !matches!(st, RingState::Live) {
            return Err(UError::from_raw(ring::err::DEAD)).annotate("ring push (state)");
        }
        self.shared.acquire();
        let code = self.shared.try_push_locked(msg);
        self.shared.release();
        if code == 0 {
            let _ = env::wake(self.key());
            Ok(())
        } else {
            Err(UError::from_raw(code)).annotate("ring push")
        }
    }

    /// 阻塞投递：满 → wait(条件键) 重试；Dead → 负码。
    pub fn push(&self, msg: &[u8]) -> UResult<()> {
        loop {
            match self.try_push(msg) {
                Ok(()) => return Ok(()),
                Err(e) if e.source.code() == ring::err::BUSY => {
                    let _ = env::wait(self.key(), usize::MAX);
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// ring 条件键（wait/wake 用；带标记位 → 内核不经 compose）。
    pub fn key(&self) -> usize {
        RING_KEY_TAG | self.id
    }
}

impl Consumer {
    /// 收取尝试（非阻塞）：成功 Ok；Err -2 = 空（Busy，wait 后重试）、-1 = Dead。
    pub fn try_pull(&self, buf: &mut [u8]) -> UResult<()> {
        self.shared.acquire();
        let code = self.shared.try_pull_locked(buf);
        self.shared.release();
        if code == 0 {
            let _ = env::wake(self.key());
            Ok(())
        } else {
            Err(UError::from_raw(code)).annotate("ring pull")
        }
    }

    /// 阻塞收取：空 → wait(条件键) 重试；Dead → 负码。
    pub fn pull(&self, buf: &mut [u8]) -> UResult<()> {
        loop {
            match self.try_pull(buf) {
                Ok(()) => return Ok(()),
                Err(e) if e.source.code() == ring::err::BUSY => {
                    let _ = env::wait(self.key(), usize::MAX);
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// ring 条件键（与 producer 同值：两端同键）。
    pub fn key(&self) -> usize {
        RING_KEY_TAG | self.id
    }
}

/// 终止（置 Dead；对端感知断开）。
pub fn close(id: usize) -> UResult<()> {
    env::ring_close(id)
}
