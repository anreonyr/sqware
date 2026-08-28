//! 用户侧 dock — 共享内存邮路（多 pier 生产 / 唯一 quay 消费）。
//!
//! 数据面 = 共享物理帧上的环形缓冲（布局契约 `ubi::dock`，与内核 mail::ring
//! 同源）。push/pull 在**本地视图基址**上原子操作：
//!   1. 取共享区自旋锁（`OFF_LOCK`，AtomicBool swap）——并发正确性集中于此；
//!   2. 状态检查（Live 才可 push；Live/Hang 可 pull）→ 满/空 → Busy（-2）；
//!   3. 写/读槽 + 单调弧推进（`write`/`read`，2 的幂掩码定位槽位）；
//!   4. 成功 → wake 条件键（dock 键带标记位）对端；阻塞侧 wait 后重试。
//!
//! 阻塞语义与 port 同构（条件循环 + 调度域 wait/wake）；断开感知 = 负码
//! Dead（-1）/ Gone（-3）。

use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

use erra::ResultExt;
use ubi::dock::{self, DOCK_KEY_TAG};
use ubi::{UError, UResult};

use crate::env;

/// dock 状态编码（与内核 `mail::ring::RingState` 同值）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockState {
    Live,
    Hang,
    Gone,
    Dead,
}

impl DockState {
    fn from_code(code: u8) -> DockState {
        match code {
            dock::state::LIVE => DockState::Live,
            dock::state::HANG => DockState::Hang,
            dock::state::GONE => DockState::Gone,
            _ => DockState::Dead,
        }
    }

    /// 是否仍可消费（pull）：Live/Hang 可取；Gone/Dead 断开。
    pub const fn pullable(self) -> bool {
        matches!(self, DockState::Live | DockState::Hang)
    }
}

/// 共享区原子视图（本地映射基址 + 固定偏移；与 ubi::dock 布局配对）。
///
/// 所有字段都经原子/volatile 访问（多任务多核共享同一物理帧）。
#[derive(Clone, Copy)]
pub(crate) struct Shared {
    base: *mut u8,
}

// SAFETY: Shared 持有的 base 是用户空间已映射 VA；原子访问无 Rust 别名，多线程
// 经原子协议同步——Send/Sync 安全（对齐保证：base 页对齐、偏移按类型对齐）。
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

impl Shared {
    /// 视图包装：`base` 为 dock 共享区本地映射基址（页对齐）。
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
        unsafe { &*self.field(dock::OFF_STATE) }
    }
    #[inline]
    fn lock(&self) -> &AtomicBool {
        // SAFETY: 布局契约。
        unsafe { &*self.field(dock::OFF_LOCK) }
    }
    #[inline]
    fn read(&self) -> &AtomicUsize {
        // SAFETY: 布局契约。
        unsafe { &*self.field(dock::OFF_READ) }
    }
    #[inline]
    fn write(&self) -> &AtomicUsize {
        // SAFETY: 布局契约。
        unsafe { &*self.field(dock::OFF_WRITE) }
    }
    #[inline]
    fn item_len(&self) -> usize {
        // SAFETY: 只读字段 open 定型后不变；offset 在共享区内。
        unsafe { *self.field::<usize>(dock::OFF_ITEM_LEN) }
    }
    #[inline]
    fn slots(&self) -> usize {
        // SAFETY: 只读字段 open 定型后不变。
        unsafe { *self.field::<usize>(dock::OFF_SLOTS) }
    }

    /// 槽 i 的数据指针（2 的幂掩码环绕定位）。
    #[inline]
    fn slot_ptr(&self, idx: usize) -> *mut u8 {
        let mask = self.slots() - 1;
        // SAFETY: 偏移在共享区内（buffer 起点 + 槽位）；调用方按槽长访问。
        unsafe { self.base.add(dock::OFF_BUFFER + (idx & mask) * self.item_len()) }
    }

    /// 取锁（自旋：swap 1 → 持锁）。
    #[inline]
    fn acquire(&self) {
        while self.lock().swap(true, Ordering::Acquire) {
            // 自旋等待（教材级：临界区极短，无公平/饥饿处理）
            core::hint::spin_loop();
        }
    }

    /// 放锁。
    #[inline]
    fn release(&self) {
        self.lock().store(false, Ordering::Release);
    }

    /// 投递尝试（锁内）：满 → Busy；Dead/Gone → 负码。返回错误码（0 = 成功）。
    fn try_push_locked(&self, msg: &[u8]) -> isize {
        debug_assert_eq!(msg.len(), self.item_len(), "dock push message size");
        let w = self.write().load(Ordering::Acquire);
        let r = self.read().load(Ordering::Acquire);
        if w.wrapping_sub(r) == self.slots() {
            return dock::err::BUSY;
        }
        // 写槽数据（先数据后发布弧；锁保护下顺序对 quay 端可见）
        // SAFETY: slot_ptr 指向映射内槽位；len == item_len 恒定。
        unsafe {
            ptr::copy_nonoverlapping(msg.as_ptr(), self.slot_ptr(w), msg.len());
        }
        self.write().fetch_add(1, Ordering::Release);
        0
    }

    /// 收取尝试（锁内）：空 → Busy；断开 → 负码；Hang 取空 → CAS Gone 钉连。
    fn try_pull_locked(&self, buf: &mut [u8]) -> isize {
        debug_assert_eq!(buf.len(), self.item_len(), "dock pull buffer size");
        let w = self.write().load(Ordering::Acquire);
        let r = self.read().load(Ordering::Acquire);
        if w.wrapping_sub(r) == 0 {
            // 空：Hang（余信取尽）→ 钉连 Gone；Live 空 → Busy（等投递）
            let st = self.state().load(Ordering::Acquire);
            if DockState::from_code(st) == DockState::Hang {
                let _ = self.state().compare_exchange(
                    dock::state::HANG,
                    dock::state::GONE,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
                return dock::err::GONE;
            }
            return dock::err::BUSY;
        }
        // 读槽数据（发布弧已保证数据可见）
        // SAFETY: slot_ptr 指向映射内槽位；len == item_len 恒定。
        unsafe {
            ptr::copy_nonoverlapping(self.slot_ptr(r), buf.as_mut_ptr(), buf.len());
        }
        self.read().fetch_add(1, Ordering::Release);
        0
    }
}

/// 生产端句柄（可复制多份；计数经内核 RingClone/RingDrop 维护）。
pub struct Pier {
    id: usize,
    shared: Shared,
}

/// 消费端句柄（唯一；离场 → Dead）。
pub struct Quay {
    id: usize,
    shared: Shared,
}

/// 端（RingJoin/RingDrop 的 side 参数）。
pub use ubi::dock::side;

/// 建 dock：返回 (Pier, Quay) 两端同源（同视图基址）。id = dock 全局 id。
pub fn open(item_len: usize, slots: usize) -> UResult<(Pier, Quay)> {
    let (id, view) = env::dock_open(item_len, slots)?;
    let shared = Shared::new(view);
    Ok((Pier { id, shared }, Quay { id, shared }))
}

/// pier 复制：内核开新计数并登记给当前任务（`Drop` 时递减）。
impl Clone for Pier {
    fn clone(&self) -> Pier {
        let _ = env::dock_clone(self.id);
        Pier {
            id: self.id,
            shared: self.shared.clone(),
        }
    }
}

/// quay/pier 释放：内核递减计数（pier 归零 → Hang；quay → Dead）。
impl Drop for Pier {
    fn drop(&mut self) {
        let _ = env::dock_drop(self.id, side::PIER);
    }
}
impl Drop for Quay {
    fn drop(&mut self) {
        let _ = env::dock_drop(self.id, side::QUAY);
    }
}

impl Pier {
    /// 投递尝试（非阻塞）：成功 Ok；Err -2 = 满（Busy，wait 后重试）、-1 = Dead。
    pub fn try_push(&self, msg: &[u8]) -> UResult<()> {
        // 状态预检（锁外读，防持锁自旋过久；锁内再校验）
        let st = DockState::from_code(self.shared.state().load(Ordering::Acquire));
        if !matches!(st, DockState::Live) {
            return Err(UError::from_raw(dock::err::DEAD)).annotate("dock push (state)");
        }
        self.shared.acquire();
        let code = self.shared.try_push_locked(msg);
        self.shared.release();
        if code == 0 {
            let _ = env::wake(self.key());
            Ok(())
        } else {
            Err(UError::from_raw(code)).annotate("dock push")
        }
    }

    /// 阻塞投递：满 → wait(条件键) 重试；Dead → 负码。
    pub fn push(&self, msg: &[u8]) -> UResult<()> {
        loop {
            match self.try_push(msg) {
                Ok(()) => return Ok(()),
                Err(e) if e.source.code() == dock::err::BUSY => {
                    let _ = env::wait(self.key(), usize::MAX);
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// dock 条件键（wait/wake 用；带标记位 → 内核不经 compose）。
    pub fn key(&self) -> usize {
        DOCK_KEY_TAG | self.id
    }
}

impl Quay {
    /// 收取尝试（非阻塞）：成功 Ok；Err -2 = 空（Busy，wait 后重试）、-1 = Dead、
    /// -3 = Gone（Hang 取空钉连，连接自然终了）。
    pub fn try_pull(&self, buf: &mut [u8]) -> UResult<()> {
        self.shared.acquire();
        let code = self.shared.try_pull_locked(buf);
        self.shared.release();
        if code == 0 {
            let _ = env::wake(self.key());
            Ok(())
        } else {
            Err(UError::from_raw(code)).annotate("dock pull")
        }
    }

    /// 阻塞收取：空 → wait(条件键) 重试；Dead/Gone → 负码。
    pub fn pull(&self, buf: &mut [u8]) -> UResult<()> {
        loop {
            match self.try_pull(buf) {
                Ok(()) => return Ok(()),
                Err(e) if e.source.code() == dock::err::BUSY => {
                    let _ = env::wait(self.key(), usize::MAX);
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// dock 条件键（与 pier 同值：两端同键）。
    pub fn key(&self) -> usize {
        DOCK_KEY_TAG | self.id
    }
}

/// 终止（置 Dead；对端感知断开）。
pub fn shut(id: usize) -> UResult<()> {
    env::dock_shut(id)
}

/// 加入已有 dock（跨 team / 复接）：返回本地 pier 或 quay 视图基址。
/// `side` = [`side::PIER`] 或 [`side::QUAY`]；quay 被占 → Busy（-2）。
pub fn join(id: usize, side: usize) -> UResult<usize> {
    env::dock_join(id, side)
}