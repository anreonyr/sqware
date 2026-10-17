//! 控制者与远端读者之间唯一的两个方向：一段共享窗。
//!
//! **布局只有这一处**：`hotpath.rs` 那一段汇编的每个偏移都取自本文件的
//! `core::mem::offset_of!(Window, …)`，故这里加一格不会与汇编悄悄错位。
//!
//! 每一格都是 `AtomicUsize`。汇编那一侧按同一份布局用 `ld`/`sd` 存取：**发布前**
//! 一次 `fence rw, rw`，**观察到的位**才当值用——与 Rust 这一侧的 Release/Acquire
//! 是同一件事实的两种写法（汇编不引入第二套可见性规则）。
//!
//! 谁写什么：控制者在放行读者**之前**写死前三格（那时还没有并发）；
//! 读者在窗口里写中间那批并把 `status` 当结论；控制者只在 `status` 的 Acquire
//! 之后读它们。两枚门各只带一位。

use core::sync::atomic::{AtomicUsize, Ordering};

/// 那一台机器与本景之间唯一共享的那一页事实。
#[repr(C)]
pub struct Window {
    /// 源页在读者那个域里的地址（`Source::page(0)`）。
    pub(super) va: AtomicUsize,
    /// 热身那次访存该读到的字节（控制者已填好的花纹）。
    pub(super) want: AtomicUsize,
    /// 读者那一段热窗口的期限（`rdtime` 的 tick 数），由时基频率折出来。
    pub(super) budget: AtomicUsize,

    /// 热身那次访存实际读到的字节。
    pub(super) warm: AtomicUsize,
    /// 热身那次访存的指令地址（本景自己那个标号）。
    pub(super) warm_at: AtomicUsize,
    /// 采样那次访存的指令地址——它与捕获到的 `sepc` 必须同一。
    pub(super) sample_at: AtomicUsize,
    /// 读者进窗口时算出的绝对期限（现场留痕，判定不用它）。
    pub(super) deadline: AtomicUsize,
    /// 读者进窗口时的 `sstatus`（`SIE` 那时是开的，之后被清）。
    pub(super) sstatus: AtomicUsize,
    /// 读者进来时换下的那支向量——生产 tracing 用的那一支，出口要装回。
    pub(super) stvec: AtomicUsize,
    /// 私有向量捕获到的 `scause`。
    pub(super) cause: AtomicUsize,
    /// 私有向量捕获到的 `stval`。
    pub(super) stval: AtomicUsize,
    /// 私有向量捕获到的 `sepc`。
    pub(super) sepc: AtomicUsize,
    /// 采样那次访存**真读回**的字节（只有没缺页时才有值）。
    pub(super) byte: AtomicUsize,

    /// 读者 → 控制者：热身那次访存已经真的发生过。
    pub(super) warmed: AtomicUsize,
    /// 控制者 → 读者：移交已经结算完（**成功返回之后**才开）。
    pub(super) gate: AtomicUsize,
    /// 读者 → 控制者：窗口的结论，也是那批载荷的发布位。
    pub(super) status: AtomicUsize,
}

impl Window {
    /// 还没跑出结论。
    pub const PENDING: usize = 0;
    /// 采样那次访存落进私有向量：`cause`/`stval`/`sepc` 才是判据。
    pub const TRAPPED: usize = 1;
    /// 采样那次访存读回了字节——要撤的那一页没从热 TLB 上撤下去。
    pub const STALE: usize = 2;
    /// 采样门没在期限内开（不外推成“缺页”）。
    pub const TIMEOUT: usize = 3;

    const fn new() -> Self {
        Self {
            va: AtomicUsize::new(0),
            want: AtomicUsize::new(0),
            budget: AtomicUsize::new(0),
            warm: AtomicUsize::new(0),
            warm_at: AtomicUsize::new(0),
            sample_at: AtomicUsize::new(0),
            deadline: AtomicUsize::new(0),
            sstatus: AtomicUsize::new(0),
            stvec: AtomicUsize::new(0),
            cause: AtomicUsize::new(0),
            stval: AtomicUsize::new(0),
            sepc: AtomicUsize::new(0),
            byte: AtomicUsize::new(0),
            warmed: AtomicUsize::new(0),
            gate: AtomicUsize::new(0),
            status: AtomicUsize::new(0),
        }
    }

    /// 控制者：把这一轮那三格写死，并把两枚门与结论拨回未开。
    ///
    /// 调用点在放行读者之前——那时还没有第二台读者，故这些格子是普通写。
    pub fn prepare(&self, va: usize, want: usize, budget: usize) {
        self.va.store(va, Ordering::Relaxed);
        self.want.store(want, Ordering::Relaxed);
        self.budget.store(budget, Ordering::Relaxed);
        for flag in [&self.warmed, &self.gate, &self.status] {
            flag.store(0, Ordering::Relaxed);
        }
    }

    /// 读者那段汇编自己按这一格取地址。
    pub fn va(&self) -> usize {
        self.va.load(Ordering::Relaxed)
    }

    pub fn want(&self) -> usize {
        self.want.load(Ordering::Relaxed)
    }

    pub fn warm(&self) -> usize {
        self.warm.load(Ordering::Relaxed)
    }

    pub fn warm_at(&self) -> usize {
        self.warm_at.load(Ordering::Relaxed)
    }

    pub fn sample_at(&self) -> usize {
        self.sample_at.load(Ordering::Relaxed)
    }

    pub fn deadline(&self) -> usize {
        self.deadline.load(Ordering::Relaxed)
    }

    pub fn sstatus(&self) -> usize {
        self.sstatus.load(Ordering::Relaxed)
    }

    pub fn stvec(&self) -> usize {
        self.stvec.load(Ordering::Relaxed)
    }

    pub fn cause(&self) -> usize {
        self.cause.load(Ordering::Relaxed)
    }

    pub fn stval(&self) -> usize {
        self.stval.load(Ordering::Relaxed)
    }

    pub fn sepc(&self) -> usize {
        self.sepc.load(Ordering::Relaxed)
    }

    pub fn byte(&self) -> usize {
        self.byte.load(Ordering::Relaxed)
    }

    /// 控制者：热身那一下已经发生过了吗。
    pub fn warmed(&self) -> bool {
        self.warmed.load(Ordering::Acquire) != 0
    }

    /// 控制者：移交成功返回之后才开这一枚。
    pub fn open_sample(&self) {
        self.gate.store(1, Ordering::Release);
    }

    /// 控制者：读者的结论；它一非零，读者写下的那批格子就都读得到了。
    pub fn status(&self) -> usize {
        self.status.load(Ordering::Acquire)
    }
}

/// 本景那一段共享窗：**只有一个读者、一次窗口**。
///
/// 每一格都是 `AtomicUsize`，故它本来就是 `Sync`；两侧的读写次序由上面那两枚门与
/// 两侧的 fence/Release/Acquire 说清。
pub static WINDOW: Window = Window::new();
