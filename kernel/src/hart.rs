// hart — 每核的运行时上下文（`tp` 指向的那块）与**核的身份**。
//
// 与 `platform::machine` 的分工：那个答"这台机器是什么"（几核、多少内存——注入
// 一次的纯值）；本模块答"**我是哪一号核**"（运行时读 `tp`，不是机器属性），以及
// 每核那块可变上下文（帧 VA / 调度器指针 / 宿住租约）。号成类型 [`HartId`]；
// "几核"是**元数**，走 `hart_count() -> usize`——数不装号。
//
// 槽数上限 `MAX_HART_SLOTS` **不在这里**：它自述是"VA 布局表达上限"（hart 帧区与
// per-hart trap 栈窗口的宽度），单一事实源在 `crate::layout`。

use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

use crate::layout::{HART_FRAME_BASE, MAX_HART_SLOTS};
use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::VirtAddr;
use crate::platform::machine;

/// 核的身份 —— hart 号。固件经 HSM `hart_start` / DTB 序授予，**内核不自铸**。
///
/// 号即槽：`PER_HART` 下标、trap 栈段号、`WAITING` 与 halt 位图的位、trace 环槽
/// 都由它派生——五种角色过去共用一个 `usize`（号 / 元数 / 位 / 槽 / 位置），
/// 故本类型只留三个出口：
///
/// - [`HartId::new`] —— 边界铸造（boot 汇编传入的 hartid、DTB 序、槽循环）；
/// - [`HartId::get`] —— 边界裸值（ABI 字段 `PerHart.id`、IPI 掩码寄存器、
///   数组下标、诊断与 wire 形状）；
/// - [`HartId::bit`] —— **号→位的唯一出口**（`1 << (h % 64)` 那类式子不再各处手写；
///   位序本身仍留 `usize`：广播掩码里的 `b` 是**位置**，不是号）。
///
/// 元数（几核）不是号：`hart_count()` 恒返 `usize`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HartId(usize);

impl HartId {
    /// 铸造：只用于「号从边界进来」的地方。
    pub const fn new(id: usize) -> Self {
        Self(id)
    }

    /// 裸值：只给边界（ABI 字段 / 固件寄存器 / 数组下标 / 诊断与 wire 形状）。
    pub const fn get(self) -> usize {
        self.0
    }

    /// 号 →（掩码字序, 字内位）。IPI 掩码与位图两类消费者共用同一个换算。
    pub const fn bit(self) -> (usize, usize) {
        (
            self.0 / usize::BITS as usize,
            1usize << (self.0 % usize::BITS as usize),
        )
    }
}

impl core::fmt::Display for HartId {
    /// 裸号形态——诊断行与门的判据逐字不变（`HartId(0)` 那种 Debug 形态不进输出）。
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.fmt(f)
    }
}

/// 已启动的 hart 集合（进程级进度记录；无功能读者，保留为诊断信息）。
static STARTED_HARTS: AtomicUsize = AtomicUsize::new(1);

/// 记录某 hart 已启动（HSM `hart_start` 成功后调用）。
pub fn mark_hart_started(hart: HartId) {
    debug_assert!(
        hart.get() < MAX_HART_SLOTS,
        "hart id {hart} beyond MAX_HART_SLOTS {MAX_HART_SLOTS}"
    );
    STARTED_HARTS.fetch_max(hart.get() + 1, Ordering::Relaxed);
}

/// 实际活跃核数 = DTB 上报核数（上限 = VA 窗口槽数 MAX_HART_SLOTS）。
///
/// **动态获取**：核数完全由 DTB 决定（`Machine.hart.count`，运行时注入）。
pub fn hart_count() -> usize {
    let n = machine::info().hart.count;
    assert!(
        n <= MAX_HART_SLOTS,
        "DTB reports {n} harts, at most {MAX_HART_SLOTS} VA slots"
    );
    n
}

/// 当前 hart id（**执行本代码的核**）——与 `Machine::hart`（总核数）不同。
///
/// S-mode 读不到 M-mode 专属 CSR `mhartid`（读它触发 illegal instruction），故
/// hartid 经 `tp` 指向的 [`PerHart`] 读取：**核空间上下文**恒有 `tp` = 本 hart 的
/// `PerHart` 指针（入口与陷阱重建维护，见 `main.rs`/`_boot_entry`/`trap_handler`
/// 第 0 步；上台时只对核空间上下文补写，见 `scheduler/core/hart.rs`）。域任务的
/// `tp` 是它自己的（S 态域与 U 态域一视同仁）——本模块与 `PerHart` 的其余读点
/// 全在 `trap_handler` 第 0 步重建**之后**的内核态执行，故不受其影响。
#[inline]
pub fn hart_id() -> HartId {
    let id: usize;
    // SAFETY: 读 tp 指向的 PerHart.id（内核态 tp 恒为本 hart PerHart 指针，无副作用）。
    unsafe {
        core::arch::asm!(
            "ld {0}, 0(tp)",
            out(reg) id,
            options(nomem, nostack, preserves_flags),
        );
    }
    HartId(id)
}

/// per-hart 上下文块——内核态 tp 指向本结构（替代旧「tp 存裸 hartid」约定）。
///
/// 汇编消费端（`__core_trap`/`__restore` 定位本 hart 帧 VA、调度器经 tp 直达）
/// 按本结构裸偏移访问，布局由编译期断言锁死（`offset_of` 检查）。
///
/// 为什么是常量数组而非运行时分配：boot 汇编（`_start`/`_boot_entry`）在
/// 机器信息注入前就要设 tp，数组必须是链接期已知符号；槽数= MAX_HART_SLOTS
/// （VA 布局表达上限）；物理支撑由 boot 期 per-hart 开销校验保证（见 trap::init）。
/// 槽宽 64 B = 2⁶：boot 汇编单条 `slli` 索引（id·64），且**每核槽独占一条
/// cache 行**——`lease` 在每次 trap 都写，与邻核共享行会乒乓。
///
/// 无 `Clone`/`Copy`（`AtomicPtr` 不支持）——const 构造逐元素填数组，
/// 消费端经指针/原子访问，不整值传递。
#[repr(C, align(64))]
pub struct PerHart {
    /// 本 hart 编号（offset 0x00；`hart_id()` 读这里）。
    pub id: usize,
    /// 本 hart 帧 VA（offset 0x08；`HART_FRAME_BASE + id·PAGE`，trap 入口的帧定位）。
    pub frame: VirtAddr,
    /// 本 hart 调度器指针（offset 0x10；boot 期 `scheduler::boot::init` 经
    /// [`set_scheduler`] 原子 store——调度器在堆上动态分配，运行时才知道地址，
    /// 故为 PerHart 唯一运行时填充字段；`current()` 经 tp 直达零索引）。
    pub scheduler: AtomicPtr<()>,
    /// 本 hart 宿住槽（offset 0x18；见 `memory::manager::asid`）：本核写
    /// （[`lease_store`]）、他核读（[`lease_load`]），静态初值 = 退租。
    pub lease: AtomicUsize,
    /// 槽对齐保留（offset 0x20..0x40）：凑 64 B 使 boot 汇编 `slli a0, 6` 单条索引。
    _pad: [usize; 4],
}

impl PerHart {
    const fn at(id: usize) -> Self {
        Self {
            id,
            // 布局常量纯算术：帧区基址 + 槽位偏移（同 layout.rs 推导）。
            frame: VirtAddr::wrap(HART_FRAME_BASE.as_usize() + id * PAGE_SIZE),
            scheduler: AtomicPtr::new(core::ptr::null_mut()),
            lease: AtomicUsize::new(crate::memory::manager::asid::vacant()),
            _pad: [0; 4],
        }
    }
}

/// per-hart 上下文数组（4096 槽 × 64 B = 256 KiB 静态数据）。
///
/// `no_mangle`：boot 汇编经 `la t0, PER_HART` PC 相对定位（恒等映射，Bare 期
/// PC 相对即物理地址）。槽位随 `MAX_HART_SLOTS`（VA 布局表达上限）。
#[unsafe(no_mangle)]
static PER_HART: [PerHart; MAX_HART_SLOTS] = {
    // const 逐元素构造（PerHart 无 Copy，`[expr; N]` 复制初始化不可用）：
    // MaybeUninit 数组 + 循环写，全部槽位填满后 assume_init。
    let mut a: core::mem::MaybeUninit<[PerHart; MAX_HART_SLOTS]> = core::mem::MaybeUninit::uninit();
    // 逐元素写：先把数组槽指针升为 PerHart 指针（stride 同数组元素）。
    let ptr = a.as_mut_ptr().cast::<PerHart>();
    let mut i = 0;
    while i < MAX_HART_SLOTS {
        // SAFETY: 逐元素写，i 恒 < MAX_HART_SLOTS。
        unsafe { ptr.add(i).write(PerHart::at(i)) };
        i += 1;
    }
    // SAFETY: MAX_HART_SLOTS 个元素已全部写入。
    unsafe { a.assume_init() }
};

/// 指定 hart 的 PerHart 指针（`tp` 装载值 / 帧 TP 装配共用）。
#[inline]
pub fn per_hart_ptr(id: HartId) -> usize {
    debug_assert!(
        id.get() < MAX_HART_SLOTS,
        "per_hart_ptr: id {id} beyond MAX_HART_SLOTS"
    );
    core::ptr::addr_of!(PER_HART[id.get()]) as usize
}

/// boot 期填充本 hart 调度器指针（`scheduler::boot::init` 调用，每个 hart 恰好
/// 一次；Release 发布 Scheduler 构建完成——后续所有读取出现在 boot 流程之后
/// （SCHEDULERS OnceLock、任务 spawn、HSM 启动等系统级屏障之后），Relaxed 读
/// 亦见稳定值）。
pub fn set_scheduler(id: HartId, p: *mut ()) {
    debug_assert!(
        id.get() < MAX_HART_SLOTS,
        "set_scheduler: id {id} beyond MAX_HART_SLOTS"
    );
    PER_HART[id.get()].scheduler.store(p, Ordering::Release);
}

/// 执行核调度器指针（**tp 直达零索引**：`ld 0x10(tp)`，替代
/// `schedulers()[hart_id()]` 的「读 id → 数组索引 → 取元素」三步）。
///
/// # Safety
/// 仅内核态（tp 恒为本 hart PerHart 指针）调用；boot 填充后恒非空（调度器
/// 运行期必已初始化）。返回指针须由调用方 cast 回具体类型使用。
#[inline]
pub fn scheduler() -> *mut () {
    let p: usize;
    // SAFETY: 读 tp 指向的 PerHart.scheduler（内核态 tp 恒为本 hart PerHart 指针）。
    unsafe {
        core::arch::asm!(
            "ld {0}, 0x10(tp)",
            out(reg) p,
            options(nomem, nostack, preserves_flags),
        );
    }
    p as *mut ()
}

/// 执行核 trap 帧 VA（**tp 直达**：`ld 0x08(tp)`，替代
/// `HART_FRAME_BASE + hart_id()·PAGE` 的「读 id → 多重 → 加法」三步）。
///
/// 与 [`scheduler`] 同款：内核态 tp 恒为本 hart PerHart 指针，编译期断言锁
/// 偏移 0x08。消费：`arm_hart` 的 sscratch 接线（hart 0/副核统一原语，
/// 执行时 tp 即在位）、boot 副核样板读取可先经 `translate` 取本 hart 帧。
#[inline]
pub fn hart_frame() -> VirtAddr {
    let f: usize;
    // SAFETY: 读 tp 指向的 PerHart.frame（内核态 tp 恒为本 hart PerHart 指针）。
    unsafe {
        core::arch::asm!(
            "ld {0}, 0x08(tp)",
            out(reg) f,
            options(nomem, nostack, preserves_flags),
        );
    }
    VirtAddr::wrap(f)
}

/// 读 hart `hart` 的租约字（他核读，Acquire）。清退协议的唯一跨核读点。
///
/// **裸字**：这一格的值域含「退驻」哨兵，不是纯号——号的读写语义（`Option<Asid>`）
/// 归 `memory::manager::asid`（`occupy` / `vacate` / `lease`），本模块只管每核一格。
pub(crate) fn lease_load(hart: HartId) -> usize {
    debug_assert!(
        hart.get() < MAX_HART_SLOTS,
        "lease_load: hart {hart} beyond MAX_HART_SLOTS"
    );
    PER_HART[hart.get()].lease.load(Ordering::Acquire)
}

/// 写**本核**租约字（Release）。不收 hart 参数——签名即"只能写自己"。
pub(crate) fn lease_store(value: usize) {
    let me = hart_id();
    PER_HART[me.get()].lease.store(value, Ordering::Release);
}

/// 编译期断言：PerHart 布局即 ABI（trap 入口/`__restore` 帧定位、调度器 tp 直达
/// 按偏移访问；槽宽 2⁶ 供 boot 汇编 `slli` 索引）。
const _: () = {
    assert!(core::mem::offset_of!(PerHart, id) == 0x00);
    assert!(core::mem::offset_of!(PerHart, frame) == 0x08);
    assert!(core::mem::offset_of!(PerHart, scheduler) == 0x10);
    assert!(core::mem::offset_of!(PerHart, lease) == 0x18);
    assert!(core::mem::size_of::<PerHart>() == 64);
};
