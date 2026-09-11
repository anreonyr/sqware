// 分配对象种类（fence/kind.rs）— **记账的唯一维度**。
//
// 取代原先两个并列枚举：`Class`（4 值："关机时怎么核账"）与 `OwnerKind`（2 值：
// 毒化策略 + 账键域）。它们本来是一个维度的碎片——`OwnerKind::KernelHeap` 恒等于
// 「账本侧的地址键块」，于是「这是什么」在代码里根本无处安放，实证有三：
//   · `SpaceInner::frame()` 一个标注点盖五种对象（trap 帧 / 懒页 / 堆页 / 栈 / COW）；
//   · 自检用的数据帧只能假标 `Persistent`（生命周期维度里没有它的位置）；
//   · `spare` / `trap-stack` / `hart-frame` 三种持久对象只能靠旁路字符串名区分。
//
// 每条属性由种类自带，调用点不再各自判断：
//   side()   — 记进哪张表（帧类别表 / 活块账本）；None = 未标注（两侧都可能有）
//   keys()   — 账键的域（地址 / 页索引：用户堆的键是 `(asid, 页索引)`，不是地址）
//   end()    — 关机时的**期望终值**（`fence::audit::check_baseline` 按它分组）
//   poison() — 毒化 / canary 策略，**派生**：账本侧 + 地址键（= `OwnerKind` 原先的全部内容）
//
// 编码：`repr(u8)`，0 = `Plain`（表全零 = 未标注语义）。**编号是构建期内部约定，
// 不是持久格式**（帧种类表与账本都是 boot 期重建的）——删一个种类即整体下移，
// 不留空洞（空洞就是飞线——删 `Cow` 时用的同一条理由）。`from_u8` 对未知值防御归 `Plain`
// （只失真计数维度；取还配对与种类无关）。

/// 对象种类数（statistics 的计数数组与视图按它定长）。
pub(crate) const KIND_COUNT: usize = 15;

/// 分配对象种类。帧侧 12 种、账侧 2 种、未标注 1 种（= 15）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Kind {
    /// 未标注：容器增长、大块直取、以及任何没说清自己是什么的分配。
    /// 0 = 表全零 ⇒ 这一项就是"未标注"的编码，不占额外状态。
    Plain = 0,
    // ── 帧侧：任务生命周期（end = Zero）──
    /// 线程 trap 帧（`FrameWindow::claim`）。
    Trap = 1,
    /// 懒页物化（`SpaceInner::materialize`）。
    Lazy = 2,
    /// 用户堆的物理页（`HeapWindow::allocate`）。
    Heap = 3,
    /// 任务栈（`StackWindow::claim`）。
    Stack = 4,
    /// 装载段帧（`loader`）。
    Image = 5,
    /// Pole 环页（mail 环形缓冲）。
    Ring = 6,
    // ── 帧侧：表 / 持久 / 只报数 ──
    /// 页表页（root + 中间表；关机与内核根表 walk 数核对）。
    Table = 7,
    /// trap 栈块（每核异常栈，boot 一次、永不归还）。
    TrapStack = 8,
    /// hart trap-context 帧（每核一页，boot 一次、永不归还）。
    HartFrame = 9,
    /// spare 仓块（崩溃路径专用，boot 一次、永不归还）。
    Spare = 10,
    /// 块池 prime 借页（自由周转，只报数）。
    Prime = 11,
    /// 自检数据帧（`health` 自取自还，只报数）。
    Probe = 12,
    // ── 账侧（活块账本）──
    /// `Arc<Task>` / `TaskIdent`。
    Task = 13,
    /// 用户堆账目（键 = `(asid, 页索引)`；随空间退役）。
    UserHeap = 14,
}

/// 记进哪张表。
#[cfg(feature = "audit")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    /// 帧类别表（per-page 字节）。
    Frame,
    /// 活块账本（按地址登记）。
    Ledger,
}

/// 账键的域。
#[cfg(feature = "audit")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Keys {
    /// 地址（物理地址 / 块基址）。
    Addr,
    /// 页索引（`(asid << 44) | (va >> 12)`——用户堆账目；不是地址）。
    Page,
}

/// 关机时的期望终值（`check_baseline` 按它分组判）。
#[cfg(feature = "audit")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum End {
    /// 必须归零（真泄漏判据）。
    Zero,
    /// 必须等于内核根表 walk 数。
    Walk,
    /// 必须逐个仍在手（登记表逐项核）。
    Held,
    /// 只报数，不判（周转 / 未标注）。
    Report,
    /// 随所属空间作废（`fence::retire`）。
    Retire,
}

#[cfg(feature = "audit")]
impl Kind {
    /// 全部种类（次序 = 编码次序）。
    pub(crate) const ALL: [Kind; KIND_COUNT] = [
        Kind::Plain,
        Kind::Trap,
        Kind::Lazy,
        Kind::Heap,
        Kind::Stack,
        Kind::Image,
        Kind::Ring,
        Kind::Table,
        Kind::TrapStack,
        Kind::HartFrame,
        Kind::Spare,
        Kind::Prime,
        Kind::Probe,
        Kind::Task,
        Kind::UserHeap,
    ];

    /// 记进哪张表；`None` = 未标注（两侧都可能有）。
    pub(crate) fn side(self) -> Option<Side> {
        match self {
            Kind::Plain => None,
            Kind::Trap
            | Kind::Lazy
            | Kind::Heap
            | Kind::Stack
            | Kind::Image
            | Kind::Ring
            | Kind::Table
            | Kind::TrapStack
            | Kind::HartFrame
            | Kind::Spare
            | Kind::Prime
            | Kind::Probe => Some(Side::Frame),
            Kind::Task | Kind::UserHeap => Some(Side::Ledger),
        }
    }

    /// 账键的域。
    pub(crate) fn keys(self) -> Keys {
        match self {
            Kind::UserHeap => Keys::Page,
            _ => Keys::Addr,
        }
    }

    /// 关机期望终值。
    pub(crate) fn end(self) -> End {
        match self {
            Kind::Trap
            | Kind::Lazy
            | Kind::Heap
            | Kind::Stack
            | Kind::Image
            | Kind::Ring
            | Kind::Task => End::Zero,
            Kind::Table => End::Walk,
            Kind::TrapStack | Kind::HartFrame | Kind::Spare => End::Held,
            Kind::Prime | Kind::Probe | Kind::Plain => End::Report,
            Kind::UserHeap => End::Retire,
        }
    }

    /// 毒化 / canary 策略（**派生**，不是第二张表）：**地址键 ∧ 非帧侧**。
    /// 用户堆（页索引键）维持清零语义、不 poison、不设 canary；帧从不 poison。
    /// `Plain`（未标注）也算：账本里未标注的记录恒是内核堆块（用户堆一律标
    /// `UserHeap`）——第一版把它漏掉，boot 三源核对当场报出
    /// "user-heap record VA on non-held page"（是假报）。
    pub(crate) fn poison(self) -> bool {
        self.keys() == Keys::Addr && self.side() != Some(Side::Frame)
    }

    /// 报表用名（小写单词，需要时以 `-` 连接——与登记名同形）。
    pub(crate) fn name(self) -> &'static str {
        match self {
            Kind::Plain => "plain",
            Kind::Trap => "trap",
            Kind::Lazy => "lazy",
            Kind::Heap => "heap",
            Kind::Stack => "stack",
            Kind::Image => "image",
            Kind::Ring => "ring",
            Kind::Table => "table",
            Kind::TrapStack => "trap-stack",
            Kind::HartFrame => "hart-frame",
            Kind::Spare => "spare",
            Kind::Prime => "prime",
            Kind::Probe => "probe",
            Kind::Task => "task",
            Kind::UserHeap => "user-heap",
        }
    }
}

impl Kind {
    /// 表值还原。未知值（元数据损坏）防御归 `Plain`。
    pub(crate) fn from_u8(v: u8) -> Kind {
        match v {
            0 => Kind::Plain,
            1 => Kind::Trap,
            2 => Kind::Lazy,
            3 => Kind::Heap,
            4 => Kind::Stack,
            5 => Kind::Image,
            6 => Kind::Ring,
            7 => Kind::Table,
            8 => Kind::TrapStack,
            9 => Kind::HartFrame,
            10 => Kind::Spare,
            11 => Kind::Prime,
            12 => Kind::Probe,
            13 => Kind::Task,
            14 => Kind::UserHeap,
            _ => Kind::Plain,
        }
    }
}
