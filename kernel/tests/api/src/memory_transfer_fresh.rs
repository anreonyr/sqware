//! 本景那一页**握手页**的线格式：控制单元与内核夹具共用的唯一一处定义。
//!
//! 两侧都不认识对方的类型：只按这里的字节偏移读写同一页里的 `u64`。页由控制单元
//! 按 [`VA`] 定点挂在自己的空间里；内核那侧不取 PA，走 `Space::copy_in/copy_out`
//! 的软件走表访问。
//!
//! 这一页只承载两件事实：控制单元请求内核夹具做哪一件事（[`off::MODE`] ＋
//! [`off::VIEW`]），以及它做到了没有（[`off::STATE`]）。内核在占槽之后读前两件、
//! 放开读者（[`off::START`]）、注入一次失败的撤页确认，再把后一件写回来。

/// 握手页钉在这个地址上：控制单元挂在这里，内核夹具按同一个数寻址。
pub const VA: usize = 0x00F0_0000;

/// 一整页里只有头这一小段被用到。
pub const WORDS: usize = 7;

/// 控制单元写进 [`off::TAG`] 的场景标记：内核夹具见到它才认这一景。
pub const TAG: u64 = 0x4652_4553_4800_0001;

/// 内核夹具放开读者时写进 [`off::START`] 的那一枚字。
pub const RELEASE: u64 = 1;

/// 各字的字节偏移。
pub mod off {
    pub const TAG: usize = 0;
    pub const MODE: usize = 8;
    pub const START: usize = 16;
    pub const VIEW: usize = 24;
    pub const STATE: usize = 32;
    pub const READY: usize = 40;
    /// 读者读到的那个数（零页就是 `0`）。
    pub const VERDICT: usize = 48;
}

/// 读者还没报读数时 [`off::VERDICT`] 上的那一格。
pub const VERDICT_NONE: u64 = u64::MAX;

/// 控制单元请内核夹具做的事。
pub mod mode {
    /// 这一景没在跑：内核夹具什么都不做。
    pub const OFF: u64 = 0;
    /// 放开读者、等它真登记在目标关系上，再注入一次失败。
    pub const AWAIT: u64 = 1;
    /// 没有等待者：占槽之后当场注入一次失败。
    pub const LATE: u64 = 2;
}

/// 内核夹具做到哪一步了。
pub mod state {
    pub const IDLE: u64 = 0;
    /// 控制单元已经把这一次的请求写好了。
    pub const PREPARED: u64 = 1;
    /// 失败注入已经装好，紧接着就返回。
    pub const ARMED: u64 = 2;
    /// 有界自旋没等到读者登记：这一次不注入。
    pub const TIMEOUT: u64 = 3;
}
