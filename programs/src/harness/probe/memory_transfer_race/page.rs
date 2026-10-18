//! 共享控制页那一面：管理端与对端读写**同一张物理页**的那几个字。
//!
//! 页的线格式在 [`handshake`](super::handshake)（内核夹具 include 同一份文件）；本文件只把它
//! 包成“一格一格读／写”这一个动作。两侧各按**自己空间里**那条关系的起点寻址——
//! 管理端是自己挂的那个定点地址，对端是 `Open` 交回给它的那个地址；两边读写的是
//! 同一张页，故坐标不同、事实同一。
//!
//! 两侧对同一格均作 Release store / Acquire load：请求与读数都按“先载荷、后发布位”
//! 的次序写，不假设哪一台在哪颗 hart 上。

use core::sync::atomic::{AtomicU64, Ordering};

use env::PieToken;

use super::handshake;

/// 本景那一页：一个起点，加那几个字。
#[derive(Clone, Copy)]
pub struct Page {
    base: usize,
}

impl Page {
    /// 这一页在本域里的起点（两域各一个，指向同一张物理页）。
    pub const fn at(base: usize) -> Page {
        Page { base }
    }

    /// 管理端起手：把标记与请求拉平（那时还没有对端）。
    pub fn reset(&self) {
        self.put(handshake::off::TAG, 0);
        self.put(handshake::off::MODE, handshake::mode::OFF);
        self.put(handshake::off::START, 0);
        self.put(handshake::off::STATE, handshake::state::IDLE);
        self.put(handshake::off::READY, 0);
        self.put(handshake::off::PEER, 0);
        self.put(handshake::off::VA0, 0);
        self.put(handshake::off::VA1, 0);
        self.put(handshake::off::READ0, handshake::NONE);
        self.put(handshake::off::READ1, handshake::NONE);
        self.put(handshake::off::PAUSE, 0);
        self.put(handshake::off::GO, 0);
        self.put(handshake::off::RELEASED, handshake::NONE);
    }

    /// 管理端：把这一次的请求摆好（标记、模式、状态，放行位拉回未开）。
    ///
    /// 最后写的才是 [`handshake::off::MODE`]：内核夹具按它认“请求已就绪”。
    pub fn request(&self, mode: u64) {
        self.put(handshake::off::START, 0);
        self.put(handshake::off::STATE, handshake::state::PREPARED);
        self.put(handshake::off::TAG, handshake::TAG);
        self.put(handshake::off::MODE, mode);
    }

    /// 管理端：请内核夹具在**下一次拆图**上停一下（一次性请求）。
    ///
    /// 对端的 `Release` 在它那个 hart 上跑；本端只摆请求，不碰对端的表。
    pub fn request_teardown(&self) {
        self.put(handshake::off::START, 0);
        self.put(handshake::off::STATE, handshake::state::PREPARED);
        self.put(handshake::off::GO, 0);
        self.put(handshake::off::RELEASED, handshake::NONE);
        self.put(handshake::off::PAUSE, 1);
        self.put(handshake::off::TAG, handshake::TAG);
        self.put(handshake::off::MODE, handshake::mode::TEARDOWN);
    }

    /// 管理端：这一景收工，内核夹具不再插手之后的任何一次移交。
    pub fn retire(&self) {
        self.put(handshake::off::MODE, handshake::mode::OFF);
    }

    /// 内核夹具做到哪一步了。
    pub fn state(&self) -> u64 {
        self.word(handshake::off::STATE)
    }

    /// 本端请内核夹具做的事（对端按它挑自己要进哪一段）。
    pub fn mode(&self) -> u64 {
        self.word(handshake::off::MODE)
    }

    /// 内核夹具在窗口里读到的那几个数（本景只入判定）。
    pub fn press_before(&self) -> u64 {
        self.word(handshake::off::PRESS_BEFORE)
    }

    pub fn press_after(&self) -> u64 {
        self.word(handshake::off::PRESS_AFTER)
    }

    pub fn peer_before(&self) -> u64 {
        self.word(handshake::off::PEER_BEFORE)
    }

    pub fn peer_after(&self) -> u64 {
        self.word(handshake::off::PEER_AFTER)
    }

    /// 那一次拆图真的从暂停里放行回来了吗（1 = 回来了）。
    pub fn resumed(&self) -> u64 {
        self.word(handshake::off::RESUMED)
    }

    /// 对端报回来的那一次 `Release` 的结果码（还没报就是 [`handshake::NONE`]）。
    pub fn released(&self) -> u64 {
        self.word(handshake::off::RELEASED)
    }

    /// 对端：报那一次 `Release` 的结果码（0 = 成功）。
    pub fn publish_released(&self, code: u64) {
        self.put(handshake::off::RELEASED, code);
    }

    /// 对端：报它手里那枚**来源**子分支的号（拆图探针按它认目标）。
    pub fn publish_source_token(&self, token: PieToken) {
        self.put(handshake::off::SOURCE_TOKEN, token.get() as u64);
    }

    /// 内核夹具在窗口里看到的：那条**别人空间**的关系在源页上真握着一条翻译。
    pub fn foreign(&self) -> u64 {
        self.word(handshake::off::FOREIGN)
    }

    /// 内核夹具那一步的名字（只入控制台，不进判定）。
    pub fn state_name(&self) -> &'static str {
        match self.state() {
            handshake::state::IDLE => "idle",
            handshake::state::PREPARED => "prepared",
            handshake::state::ARMED => "armed",
            handshake::state::TIMEOUT => "timeout",
            handshake::state::PAUSED => "paused",
            _ => "?",
        }
    }

    /// 对端：报它就绪——先写载荷（tid、两个 VA、第 0 页的读数），最后才写
    /// [`handshake::off::READY`]；管理端看到 `READY` 时那几格已经能看见。
    pub fn report(&self, peer: usize, va0: usize, va1: usize, read0: u64) {
        self.put(handshake::off::PEER, peer as u64);
        self.put(handshake::off::VA0, va0 as u64);
        self.put(handshake::off::VA1, va1 as u64);
        self.put(handshake::off::READ0, read0);
        self.put(handshake::off::READY, 1);
    }

    pub fn ready(&self) -> u64 {
        self.word(handshake::off::READY)
    }

    pub fn peer(&self) -> u64 {
        self.word(handshake::off::PEER)
    }

    pub fn va0(&self) -> u64 {
        self.word(handshake::off::VA0)
    }

    pub fn va1(&self) -> u64 {
        self.word(handshake::off::VA1)
    }

    pub fn read0(&self) -> u64 {
        self.word(handshake::off::READ0)
    }

    /// 对端等的那一枚字：内核在占槽之后把它放开。
    ///
    /// **这是本景唯一的跨 hart 放行**：它用 `Acquire`，故对端看到 `RELEASE` 之后
    /// 那次对目标页的访存不在它之前。
    pub fn start(&self) -> u64 {
        self.word(handshake::off::START)
    }

    /// 对端：把第 1 页的读数写回（管理端按 `Acquire` 读）。
    pub fn publish_read1(&self, value: u64) {
        self.put(handshake::off::READ1, value);
    }

    /// 对端写回来的那一枚读数（还没报就是 [`handshake::NONE`]）。
    pub fn read1(&self) -> u64 {
        self.word(handshake::off::READ1)
    }

    /// 这一页上第 `at` 个字节处那一格。
    fn cell(&self, at: usize) -> &'static AtomicU64 {
        // SAFETY: 本景这一页按页对齐、在自己空间的映射里，故 `base + at` 是指向有效
        // `u64` 的 8 字节对齐地址（`at` 取自 [`handshake::off`]）；它括到本域走完，
        // 且两个方向都只按同一份线格式的那几个字寻址。
        unsafe { &*((self.base + at) as *const AtomicU64) }
    }

    /// 发布一格：另一颗 hart 看到它时，本次发布之前的写都已经能看见。
    fn put(&self, at: usize, value: u64) {
        self.cell(at).store(value, Ordering::Release);
    }

    /// 读一格：读到别人发布的那个值之后，本次读之前的那些写都已经能看见。
    fn word(&self, at: usize) -> u64 {
        self.cell(at).load(Ordering::Acquire)
    }
}
