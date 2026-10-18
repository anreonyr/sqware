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
//!   第二段（拆图那一手）由它把 [`off::PAUSE`] 拉成一次性请求；
//! - 内核夹具：占槽之后把 [`off::START`] 放开（唯一那次跨 hart 放行）、把
//!   [`off::STATE`] 写成它做到了哪一步；拆图那一段里它把 [`off::GO`] 放开、
//!   把读到的那几个数写成 [`off::PRESS_BEFORE`]／[`off::PRESS_AFTER`]／
//!   [`off::PEER_BEFORE`]／[`off::PEER_AFTER`]；
//! - 对端：把它的读数与坐标写回（[`off::READY`] 及其后那几格），
//!   拆图那一段末尾把 [`off::RELEASED`] 写成那一次 `Release` 的结果码；
//!   它交出的那枚子分支的号（[`off::SOURCE_TOKEN`]）也在这里报回来。

/// 共享控制页钉在这个地址上：管理端挂在这里，内核夹具按同一个数寻址。
pub const VA: usize = 0x00F0_0000;

/// 一整页里只有头这一小段被用到。
pub const WORDS: usize = 20;

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
    /// 管理端：请内核夹具在这一次拆图上停一下（一次性，夹具读走就清零）。
    pub const PAUSE: usize = 88;
    /// 内核夹具：放行那一次拆图（唯一那次把 `finish` 放开的写）。
    pub const GO: usize = 96;
    /// 对端：那一次真 `Release` 的结果码（还没报就是 [`NONE`]，`0` = 成功）。
    pub const RELEASED: usize = 104;
    /// 内核夹具在窗口里读到的：压着源页的关系数（放行之前 / 之后）。
    pub const PRESS_BEFORE: usize = 112;
    pub const PRESS_AFTER: usize = 120;
    /// 内核夹具在窗口里读到的：对端那个关系在源页 VA 上还有没有翻译
    /// （1 = 还有）（放行之前 / 之后）。
    pub const PEER_BEFORE: usize = 128;
    pub const PEER_AFTER: usize = 136;
    /// 内核夹具：那一次拆图真的从暂停里放行回来了（1 = 回来了）。
    pub const RESUMED: usize = 144;
    /// 对端报回来的那枚**来源**子分支的号：拆图探针按它认“拆的就是本景选定那一条”，
    /// 而不是只按形状认。
    pub const SOURCE_TOKEN: usize = 152;
}

/// 管理端请内核夹具做的事。
pub mod mode {
    /// 这一景没在跑：内核夹具什么都不做。
    pub const OFF: u64 = 0;
    /// 放开对端、等它真登记在**别人空间**那条关系上。
    pub const FAULT: u64 = 1;
    /// 对端释放它那枚来源子分支（`Cleanup::finish`）与在途窗口重叠。
    pub const TEARDOWN: u64 = 2;
}

/// 内核夹具做到哪一步了。
pub mod state {
    pub const IDLE: u64 = 0;
    /// 管理端已经把这一次的请求摆好了。
    pub const PREPARED: u64 = 1;
    /// 在途窗口里真看到了对端的登记（或真看到了那一次拆图落地），紧接着就返回。
    pub const ARMED: u64 = 2;
    /// 有界自旋没等到：这一次的结论按超时报，不冒充做到过。
    pub const TIMEOUT: u64 = 3;
    /// 拆图停在 `finish` 入口（清理已经取得、还没落地），等窗口里那一手放行。
    pub const PAUSED: u64 = 4;
}
