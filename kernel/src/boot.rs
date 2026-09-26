// 启动（boot）— 把 work 子系统拉起到首个用户任务。

use core::arch::global_asm;

use alloc::format;
use alloc::vec;
use riscv::register::satp;

use crate::console::Sink;
use crate::hart::{self, HartId};
use crate::layout::{HART_FRAME_BASE, TRAP_STACK_SLOT_SIZE};
use crate::layout::{ROOT_STACK_CANARY, root_stack_base};
use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::mode;
use crate::platform::machine;
use crate::runtime::diagnose::report::Report;
use crate::runtime::diagnose::trace;
use crate::runtime::switcher::context::TrapContext;
use crate::runtime::switcher::trampoline::{alltraps_va, restore};
use crate::runtime::switcher::trap::{arm_hart, trap_stack, trap_stack_base, trap_stack_edge};
use crate::work::room::scheduler;
use crate::work::unit::team::kernel;

global_asm!(
    ".section .text.boot",
    ".align 2",
    ".globl _boot_entry",
    "_boot_entry:",
    "    la   t0, PER_HART", // &PER_HART[0]（恒等映射，Bare 下 PC 相对即物理地址）
    "    slli t1, a0, 6",    // a0 = hartid（HSM Start 传入）· 64（PerHart 槽宽 2⁶）
    "    add  tp, t0, t1",   // tp = 本 hart PerHart 指针（入口约定，见 `hart_id()`）
    "    csrc sstatus, 2",
    "    mv   sp, a1", // opaque = 本 hart trap 栈顶（HSM Start 传入）
    "    call boot_main",
);

unsafe extern "C" {
    /// 副核入口（HSM Start 的 start_addr；内核镜像恒等加载，链接地址即物理地址）。
    static _boot_entry: u8;
}

/// SBI 式启动横幅：机器/板级 + 陷阱布局两块投稿成一个 banner 段落；值用
/// format! 拼装。
pub fn banner() {
    let m = machine::info();
    let mut r = Report::default();
    {
        let p = r.paragraph("banner", None);
        for (label, value) in [
            ("hart count", format!("{} H", m.hart.count)),
            ("hart this", format!("{}", hart::hart_id())),
            ("timebase", format!("{} Hz", m.hart.hertz)),
            (
                "dram",
                format!("{:#x}..{:#x}", m.dram.base, m.dram.range().end),
            ),
            (
                "free",
                format!("{:#x}..{:#x}", m.free.base, m.free.range().end),
            ),
            ("trap vector", format!("{:#x}", alltraps_va())),
            (
                "kernel frames",
                format!(
                    "{:#x}..{:#x}",
                    HART_FRAME_BASE.as_usize(),
                    HART_FRAME_BASE.as_usize() + m.hart.count * PAGE_SIZE
                ),
            ),
            (
                "trap stack",
                format!(
                    "{:#x}..{:#x}",
                    trap_stack_base(HartId::new(0)).as_usize(),
                    trap_stack_edge(HartId::new(0)).as_usize()
                ),
            ),
            (
                "trap stack this",
                format!(
                    "{} @ {:#x}..{:#x}",
                    hart::hart_id(),
                    trap_stack_base(hart::hart_id()).as_usize(),
                    trap_stack_edge(hart::hart_id()).as_usize()
                ),
            ),
        ] {
            p.items.push(vec![Some(label.into()), Some(value)]);
        }
    }
    let sealed = r.seal();
    let mut sink = Sink;
    crate::runtime::diagnose::render::render(sealed, &mut sink, 0);
}

/// 装出世界：装配钩子与锁序 → 自检（debug 档）→ 造根服务域（`spawn_root`）
/// → HSM 拉起副核。到「root 已产、副核已起」为止。**返回**——起跑留给 [`run`]。
pub fn init() {
    // per-hart 调度器状态按实际核数（DTB）动态分配——先于任何调度器访问
    scheduler::boot::init();

    // 钩子注册（一次性；顺序即 halt 时执行顺序）：
    //   1) 调度器就绪队列 + messenger 簿记的强引用归还（`scheduler::rip`）
    //   2) block 池冲洗（所有 Arc 已归还后帧基线才稳定）
    // mail 的资源不再有自己的关机钩子：它随 `Task::drop` 链透传（`PoleMeta::drop`
    // 还物理帧）。exit 钩子（每条 reaped 任务）：messenger::doom + gate::doom。
    register_runtime_hooks();

    // lockdep 装配（debug 构建）：per-hart 持有集。release 为 no-op。
    // 置于调度器就绪后、spawn 演示任务/HSM 拉起副核前——正是多核 ABBA 的生效窗口。
    #[cfg(debug_assertions)]
    crate::lock::init_depend(hart::hart_count()).expect("depend init failed");

    // 健康检查：`debug` 档在**启动期**静默跑一遍八例（失败即 panic → crash scene）。
    // 用例的**登记与逐例打点**不在核心里了——那是 `kernel/tests/embedded.rs` 的事
    // （用户裁定"迁移到 embedded-test"）。
    //
    // 同位置、同时点：调度器已就绪、`spawn_root` 未起 ⇒ 用例**没有 shell、没有装槽**，
    // 只有单核与早启动期设施（`putln!`、块/frame 分配器、页表树、`Space` 原语）。
    crate::health::run();

    // **测试模式必须有一张镜像**：没有它 `spawn_root` 会返 `Ok(None)`（"无任务照样能
    // 停机"那条路照走），于是那一例**静默地什么都没跑就绿了**——最难查的一类假象
    // （与 `runner.nu` 头注里"而不是静默起一台没有程序的机器"同一条纪律）。
    // panic ⇒ `testing()` 的 panic 通道 ⇒ semihosting abort ⇒ 那一例红。
    if crate::testing() && machine::info().initrd().is_none() {
        panic!(
            "整机用例没有镜像：`cargo-qtest` 不带 `-initrd`。请用 \
             `nu scripts/qtest.nu --scene <景>` 跑（本检查只在测试模式生效）"
        );
    }

    // 根任务交给信标：它一走即"会话结束"，信标据此把收尾期与会话期的空档分开
    // （见 `scheduler::core::beacon` 的头注）。
    if let Some(root) = spawn_root().expect("boot spawn failed") {
        crate::work::room::scheduler::core::beacon_arm(&root);
    }

    // 根服务已产生：标记 `ROOTED` 让 `done()` 守门放行——防 PUSHED==0 永久误判为
    // "全部结束"（此刻其实还没有任何任务）。**必须在 HSM 拉起副核之前**——副核从
    // idle() 进 run()/wait() 读 done() 时见到 true，则 PUSHED==0 立即 halt；
    // 否则一直 WFI 等不会到达的 IPI。
    crate::work::room::conductor::rooted();

    // 多核：HSM 拉起其余副核。
    boot_harts();

    // IPI 自检（debug 档）：副核已在各自 WFI 里，此刻是"一记门铃能不能叫醒它"的
    // 唯一干净时点（没有任务、没有到点登记 ⇒ 醒了只可能是那一记 IPI）。**这一趟不限**
    // ——它是本模块那张判据表的出处；负载期那次（借空闲核跑）自带 100 ms 上界，见
    // `diagnose::ipi` 的两条照实记。
    #[cfg(debug_assertions)]
    {
        crate::runtime::diagnose::ipi::run("early", None);
        crate::runtime::diagnose::ipi::start_delayed();
    }

    // ROOT 栈完整性审核：boot 期栈溢出即使未越过 guard 页（4 KiB 内）也会在此暴露。
    let boot_guard = unsafe { (root_stack_base() as *const usize).read() };
    assert!(
        boot_guard == ROOT_STACK_CANARY,
        "ROOT stack overflow during boot: canary corrupted {boot_guard:#x}",
    );

    // 首任务由 `run` 取（不能用 `spawn_root` 返回的那个帧 PA——可能已被副核 steal 走）。
}

/// 起跑：交出本 hart，直到世界收场。**不返回**。
///
/// 收场那一刀在 `conductor::halt`（产品路复位、测试路按账退出），故这一句在产品路与
/// 测试路**同一个形状**——测试用例的体就是 `boot::init(); boot::run();`。
pub fn run() -> ! {
    restore(scheduler::trap::run())
}

/// 注册关机 / 退出钩子——一次性把"哪个子系统要在什么时机清什么"的目录
/// 从 conductor / messenger 转移到此处；mail 内部不再被 core 直接命名。
fn register_runtime_hooks() {
    use crate::work::room::conductor;
    use crate::work::room::messenger;

    // 每条 reaped 任务两条级联：结构面 doom（父删子随，沿 heir 扑杀子域）+
    // 能力面 gate::doom（派生链随其断，沿 sire 反查子树）。mail 资源释放走
    // Task::drop 链透传，无需 task_exit。
    static EXIT_HOOKS: &[fn(env::TaskId)] = &[
        crate::work::room::messenger::doom,
        crate::work::unit::gate::doom,
    ];
    messenger::hook(EXIT_HOOKS);

    // 快照提供者：gate 的查询面与级联要「全世界任务」，但 gate 不依赖 scheduler
    // ——依赖倒置在此一次性接上（此后 gate::snap() 即可取快照）。
    crate::work::unit::gate::install(crate::work::room::scheduler::core::roster);

    // 关机序列：`scheduler::rip`（清任务队列 + messenger 簿记）→ mail 由
    //   drop 链透传（`PoleMeta::drop` 还物理帧）→ block 池冲洗。**不看账**：
    //   审计层的关机判词随那一层删了，这里只剩"把东西还回去"。
    const SHUTDOWN_HOOKS: &[fn()] = &[
        crate::work::room::scheduler::core::rip,
        crate::memory::allocator::block::flush,
    ];
    conductor::hook(SHUTDOWN_HOOKS);
}

/// 装出根服务域（**boot 的唯一 spawn**）：按打包期常量取 root 镜像 → `Build` 成
/// S 态域 → 把 initrd 区与**配对块**只读映射进它的空间（root 自己解析清单与设备供给）
/// → 产并放行引导线程。
///
/// 之后所有任务都由**域**产生（引导域起编排域，编排域起其余各域；`Build`/`Spawn`/`Hatch`）
/// ——内核只认识"第一个域"，不认识服务编排那件事；系统在全部任务回收后
/// 自然停机（`conductor::done`）。清单与设备语义的**解释权都在 root**——内核不含
/// 清单格式，也不解释设备。
fn spawn_root() -> Result<Option<alloc::sync::Arc<crate::work::unit::task::Task>>, MapError> {
    let Some(region) = machine::info().initrd() else {
        // 无 initrd ⇒ 没有任何任务会被产生；`conductor::done` 的 `PUSHED == 0`
        // 分支保证照样能停机。故这里返回"没有根任务"（信标不武装）。
        return Ok(None);
    };
    // initrd 区恒等映射（=物理地址），直接按其物理基址读。
    let blob: &'static [u8] =
        unsafe { core::slice::from_raw_parts(region.base as *const u8, region.size) };
    let elf = crate::platform::initrd::root_image(blob).expect("initrd: root image missing");

    // 源 = **内核直读的一块**（initrd 区恒等映射）：`Source::Slice` 的 `read` 只是切片，
    // 内核这一侧一个字节都不拷。
    let source = crate::work::unit::source::Source::Slice(elf);
    let team = crate::work::unit::build(
        &source,
        crate::work::unit::space::SpaceKind::Supervisor,
        crate::work::unit::weak::TaskWeak::empty(),
    )
    .expect("assemble root elf");

    // 清单视图：在 root 的用户段里**登记**一段 VA（lowest first-fit，紧接镜像），
    // 把 initrd 区（持久保留区，帧分配器永不动它）只读借用映射进去。VA 与长度
    // 经启动参数告知 root——内核不含清单格式。
    let view_size = region.size.next_multiple_of(PAGE_SIZE);
    let view = team.space.with_flush(
        |inner| -> Result<crate::memory::manager::addr::VirtAddr, MapError> {
            let va = inner.allocate(crate::work::unit::space::SegmentKind::Normal, view_size)?;
            inner.borrow(
                va,
                crate::memory::manager::addr::PhysAddr::from_raw(region.base),
                view_size,
                read_only(),
            )?;
            Ok(va)
        },
    )?;

    // 设备供给：**一次设备树扫描**，每台设备一枚门闩（`Payload::Region`）。
    // 扫描在 spawn 之前（条数要进启动参数），落表在 spawn 之后（门闩要落进那个
    // 刚产生的任务）——中间这一小段由本函数的局部量持着，不留内核静态。
    let devices = crate::platform::devices::scan();
    let (pairs_pa, pairs_bytes) = crate::platform::devices::block();
    let pairs = team.space.with_flush(
        |inner| -> Result<crate::memory::manager::addr::VirtAddr, MapError> {
            let va = inner.allocate(crate::work::unit::space::SegmentKind::Normal, pairs_bytes)?;
            inner.borrow(
                va,
                crate::memory::manager::addr::PhysAddr::from_raw(pairs_pa),
                pairs_bytes,
                read_only(),
            )?;
            Ok(va)
        },
    )?;

    // 引导线程：args 的布局见 `plan::args`（清单区 VA / 字节数 / 配对块 VA / 条数）。
    // 按常量逐格写，不靠位置——布局与读它的 root 是同一份定义。
    let mut args = [0usize; plan::args::LEN];
    args[plan::args::VIEW] = view.as_usize();
    args[plan::args::VIEW_LEN] = region.size;
    args[plan::args::PAIRS] = pairs.as_usize();
    args[plan::args::COUNT] = devices.len();
    let bootstrap = team.task().args(args.to_vec()).hold()?;
    // 两步分开：这段的形状就是 System Protocol 的 `Spawn`（恒产 `Held`）→ `Hatch`。
    // 中间没有要授的东西，但顺序要看得见——内核侧不再有"产并放行"的别名。
    crate::work::unit::task::Task::release(&bootstrap).expect("freshly held task must release");
    crate::platform::devices::install(&bootstrap, devices);

    #[cfg(debug_assertions)]
    team.space.audit();

    #[cfg(debug_assertions)]
    kernel().expect("kernel team not initialized").space.audit();

    Ok(Some(bootstrap))
}

/// boot 借映块统一的只读页标志（清单视图 / 配对块——同一种东西，同一份标志）。
fn read_only() -> crate::memory::manager::entry::PteFlags {
    use crate::memory::manager::entry::PteFlags;
    PteFlags::V | PteFlags::R | PteFlags::A | PteFlags::D
}

/// boot 启动：HSM `hart_start` 逐个拉起 hart 1..count-1。
///
/// start_addr = _boot_entry（恒等映射地址）；opaque = 该 hart 的 trap 栈顶
/// ——副核入口直接 `mv sp, a1`，寄存器传递，无需共享内存同步。
fn boot_harts() {
    // boot hart 不一定是 0（QEMU/OpenSBI 随机选）——它已在运行，须标记为已启动，
    // 并只 HSM 拉起**其它** hart（0..count 中除自身外全部）。
    let me = hart::hart_id();
    hart::mark_hart_started(me);
    let count = hart::hart_count();
    let entry = core::ptr::addr_of!(_boot_entry) as usize;
    for hart in 0..count {
        if HartId::new(hart) == me {
            continue;
        }
        // opaque = trap 栈物理栈顶（装配产物块基址 + 布局常量段偏移组装）
        let stack_top = trap_stack() + (hart + 1) * TRAP_STACK_SLOT_SIZE;
        // 同事件也进 trace（hart 0 窗口）：崩溃回放可见启动序列。
        trace::note(trace::EventKind::Boot(trace::BootEvent::Launch { hart }));
        let r = sbi::HsmCall::new(sbi::fid::Hsm::Start)
            .args(sbi::ecall::SArgs {
                a0: hart,
                a1: entry,
                a2: stack_top,
                ..Default::default()
            })
            .call();
        if r.is_err() {
            panic!("failed to start hart {hart}: {r:?}");
        }
        hart::mark_hart_started(HartId::new(hart));
    }
}

/// 副核主流程：per-hart CSR 配置后进入 idle。
#[unsafe(no_mangle)]
pub(crate) extern "C" fn boot_main() -> ! {
    // 副核 per-hart 初始化：先取共享内核 token（从**本 hart** 帧读——所有先上
    // 台的核都在 trap::init 填过相同的 kernel_satp，读自身帧语义最贴 per-hart；
    // 其余 per-hart CSR（stvec/sscratch/sie）与 hart 0 走同一原语 trap::arm_hart。
    let me = hart::hart_id();
    let ktc = kernel()
        .expect("kernel team not initialized")
        .space
        .translate(hart::hart_frame())
        .expect("kernel frame not mapped")
        .0;
    let frame = unsafe { &*(ktc.as_usize() as *const TrapContext) };
    let ksatp = frame.kernel_satp;
    // 探测所得模式 token：低 44 位 ppn、[63:44] asid/模式（字段访问器拆解，
    // 无裸位运算；模式位随 mode()，副核与主核同模式）
    unsafe {
        satp::set(mode::mode(), ksatp.asid(), ksatp.ppn());
        core::arch::asm!("sfence.vma");
    }
    arm_hart();
    // 登记内核驻留（ASID 0）：本核刚 `sfence.vma` 过。副核在此之前是退驻态
    // （PerHart 静态初值），不会被任何清退选中。
    crate::memory::manager::asid::occupy(crate::memory::manager::asid::Asid::kernel());
    // 启动完成写进 trace（直打控制台会扰 panic 现场）；`hart` 是导出形状，恒裸号。
    trace::note(trace::EventKind::Boot(trace::BootEvent::Done {
        hart: me.get(),
    }));
    scheduler::boot::idle()
}
