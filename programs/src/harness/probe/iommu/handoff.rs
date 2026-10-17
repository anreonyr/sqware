//! 控制者与端点服务之间那几格：**只传坐标，不传内存**。
//!
//! 控制者发现的那几个数、以及它按保留布好那几段之后的位置，服务靠这几格才知道自己在哪儿；
//! 反过来服务只回两格：设备给出的队列长度，与它自己看到的 `used.idx`。
//! 两边是同一 Team 里的一台真任务（与 `memory-transfer-tlb` 那一具同一条做法），
//! 故这几格就是共享的；布局只有这一处。
//!
//! 次序：坐标与控制者自己那两枚门都是**放开服务之前**写下的（那时还没有第二个写者），
//! 故它们是普通写；只有 `armed` 那一枚是跨任务的（Release/Acquire）：控制者摆好第二轮
//! 才让它收工，于是“服务死的时候端点是活的、队列里压着没打铃的一轮”这件事是**构造**出来的，
//! 不是碰巧。

use core::sync::atomic::{AtomicUsize, Ordering};

/// 端点服务开工前要拿到的全部坐标，加两枚跨任务的门。
pub struct Handoff {
    /// 交给服务的那一枚 BAR 窗口（Pie token）。
    pub bar: AtomicUsize,
    /// 那一枚窗口的页数：服务用它核自己的 `Open` 长度——**它拿到的只有这一段**。
    pub pages: AtomicUsize,
    /// 能力表四格在 BAR 内的偏移与通知乘数。
    pub common: AtomicUsize,
    pub notify: AtomicUsize,
    pub multiplier: AtomicUsize,
    pub device: AtomicUsize,
    /// 队列三段（设备那一侧看到的 IOVA）。
    pub desc: AtomicUsize,
    pub avail: AtomicUsize,
    pub used: AtomicUsize,
    /// 已用环在本域里的地址：服务靠它等完成记录。
    pub used_at: AtomicUsize,
    /// 控制者 → 服务：第二轮已摆好，可以收工了。
    pub armed: AtomicUsize,
    /// 服务 → 控制者：设备给出的队列长度（描述符表能放几项）。
    pub queue_size: AtomicUsize,
    /// 服务 → 控制者：它自己看到的 `used.idx`（现场，不是判据）。
    pub observed: AtomicUsize,
}

impl Handoff {
    const fn new() -> Self {
        Self {
            bar: AtomicUsize::new(0),
            pages: AtomicUsize::new(0),
            common: AtomicUsize::new(0),
            notify: AtomicUsize::new(0),
            multiplier: AtomicUsize::new(0),
            device: AtomicUsize::new(0),
            desc: AtomicUsize::new(0),
            avail: AtomicUsize::new(0),
            used: AtomicUsize::new(0),
            used_at: AtomicUsize::new(0),
            armed: AtomicUsize::new(0),
            queue_size: AtomicUsize::new(0),
            observed: AtomicUsize::new(0),
        }
    }

    /// 控制者：第二轮摆好了，服务可以收工。
    pub fn arm_exit(&self) {
        self.armed.store(1, Ordering::Release);
    }

    /// 服务：收工的许可到了吗。
    pub fn armed(&self) -> bool {
        self.armed.load(Ordering::Acquire) != 0
    }
}

/// 本景唯一的这一具交接面。
pub static HANDOFF: Handoff = Handoff::new();
