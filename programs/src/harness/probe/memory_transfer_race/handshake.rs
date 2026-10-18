//! 本景那一页**共享控制页**的线格式：管理端、对端与内核夹具共用的唯一一处定义。
//!
//! 这一页是三个方向的合流处，故它是**本景唯一那份线格式**：
//!
//! - 管理端（引导镜像那一台，自己的 Team／Space）把它按 [`VA`] 定点挂在自己空间里；
//! - 对端（被装载进另一个 Team／另一个 Space 的那一台）从**同一份来源**的
//!   子分支里把它打开，于是两侧读写的是同一张物理页；
//! - 内核夹具在真实在途窗口里按 [`VA`] 走**调用者空间**的软件走表读它。
//!
//! 它**不是**迁移范围的一部分：来源是另一份私有分配，两者各有各的寿命
//! （见 `source.rs`）。页里只有 [`off`] 那几个 `u64`，两侧都不认识对方的类型。
//!
//! 三个方向各写哪几格：
//!
//! - 管理端：起手把标记与请求摆好（[`off::TAG`]／[`off::MODE`]／[`off::STATE`]）；
//! - 内核夹具：占槽之后把 [`off::START`] 放开（唯一那次跨 hart 放行）、把
//!   [`off::STATE`] 写成它做到了哪一步；
//! - 对端：把它的读数与坐标写回（[`off::READY`] 及其后那几格）。

/// 共享控制页钉在这个地址上：管理端挂在这里，内核夹具按同一个数寻址。
pub const VA: usize = 0x00F0_0000;

/// 一整页里只有头这一小段被用到。
pub const WORDS: usize = 16;

/// 管理端写进 [`off::TAG`] 的场景标记：内核夹具见到它才认这一景。
pub const TAG: u64 = 0x5241_4345_0000_0001;

/// 内核夹具放开对端时写进 [`off::START`] 的那一枚字。
pub const RELEASE: u64 = 1;

/// 一格还没被写过时的哨兵。
pub const NONE: u64 = u64::MAX;

/// 各字的字节偏移。
pub mod off {
    pub const TAG: usize = 0;
    pub const MODE: usize = 8;
    pub const START: usize = 16;
    pub const STATE: usize = 24;
    pub const READY: usize = 32;
    pub const PEER: usize = 40;
    pub const VA0: usize = 48;
    pub const VA1: usize = 56;
    pub const READ0: usize = 64;
    pub const READ1: usize = 72;
    /// 内核夹具：那条**别人空间**的关系在源页上真握着一条翻译（1 = 是）。
    pub const FOREIGN: usize = 80;
}

/// 管理端请内核夹具做的事。
pub mod mode {
    /// 这一景没在跑：内核夹具什么都不做。
    pub const OFF: u64 = 0;
    /// 放开对端、等它真登记在**别人空间**那条关系上。
    pub const FAULT: u64 = 1;
}

/// 内核夹具做到哪一步了。
pub mod state {
    pub const IDLE: u64 = 0;
    /// 管理端已经把这一次的请求摆好了。
    pub const PREPARED: u64 = 1;
    /// 在途窗口里真看到了对端的登记，紧接着就返回。
    pub const ARMED: u64 = 2;
    /// 有界自旋没等到登记：这一次的结论按超时报，不冒充登记过。
    pub const TIMEOUT: u64 = 3;
}
