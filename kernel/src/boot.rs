// 启动（boot）— 把 work 子系统拉起到首个用户任务。

use core::arch::global_asm;

use alloc::format;
use alloc::vec;
use riscv::register::satp;

use crate::console::Sink;
use crate::layout::{HART_FRAME_BASE, TRAP_STACK_SLOT_SIZE};
use crate::machine;
use crate::machine::{ROOT_STACK_CANARY, root_stack_base};
use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::mode;
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
            ("hart this", format!("{}", machine::hart_id())),
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
                    trap_stack_base(0).as_usize(),
                    trap_stack_edge(0).as_usize()
                ),
            ),
            (
                "trap stack this",
                format!(
                    "{} @ {:#x}..{:#x}",
                    machine::hart_id(),
                    trap_stack_base(machine::hart_id()).as_usize(),
                    trap_stack_edge(machine::hart_id()).as_usize()
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

/// 启动多任务：spawn 演示团队后进入首个线程。
pub fn init() -> ! {
    // per-hart 调度器状态按实际核数（DTB）动态分配——先于任何调度器访问
    scheduler::boot::init();

    // 钩子注册（一次性；顺序即 halt 时执行顺序）：
    //   1) dock / ring 注册表清空（触发 Meta drop 归还共享区帧）
    //   2) 调度器槽载荷归还（per-hart LastIdent Arc）
    //   3) block 池冲洗（所有 Arc 已归还后帧基线才稳定）
    //   4) audit 基线核对（仅 audit feature）
    // exit 钩子（每条 reaped 任务）：dock::task_exit + ring::task_exit
    register_runtime_hooks();

    // lockdep 装配（debug 构建）：per-hart 持有集。release 为 no-op。
    // 置于调度器就绪后、spawn 演示任务/HSM 拉起副核前——正是多核 ABBA 的生效窗口。
    #[cfg(debug_assertions)]
    crate::lock::init_depend(machine::hart_count()).expect("depend init failed");

    // 健康检查：三个探针（spare 预算验收 / PT 回收自测 / 分配器压测）全在
    // `debug_assertions` 档，release 下本函数是空体。任一失败 fail-fast
    // （panic → crash scene）。
    crate::health::run();

    spawn_root().expect("boot spawn failed");

    // 根服务已产生：标记 `ROOTED` 让 `done()` 守门放行——防 PUSHED==0 永久误判为
    // "全部结束"（此刻其实还没有任何任务）。**必须在 HSM 拉起副核之前**——副核从
    // idle() 进 run()/wait() 读 done() 时见到 true，则 PUSHED==0 立即 halt；
    // 否则一直 WFI 等不会到达的 IPI。
    crate::work::room::conductor::rooted();

    // 完整性审计（audit feature，debug 恒开）：三源交叉核对 + 类别计数 sanity
    // （类别记账替代旧 boot 基线快照——见 fence/audit 模块头）。
    #[cfg(feature = "audit")]
    crate::memory::allocator::fence::audit::audit();

    // 多核：HSM 拉起其余副核。
    boot_harts();

    // ROOT 栈完整性审核：boot 期栈溢出即使未越过 guard 页（4 KiB 内）也会在此暴露。
    let boot_guard = unsafe { (root_stack_base() as *const usize).read() };
    assert!(
        boot_guard == ROOT_STACK_CANARY,
        "ROOT stack overflow during boot: canary corrupted {boot_guard:#x}",
    );

    // 进入调度：从本 hart 调度器取首任务（不能用 spawn 返回的帧 PA——可能已被
    // 副核 steal 走）
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
    static EXIT_HOOKS: &[fn(usize)] = &[
        crate::work::room::messenger::doom,
        crate::work::unit::gate::doom,
    ];
    messenger::hook(EXIT_HOOKS);

    // 快照提供者：gate 的查询面与级联要「全世界任务」，但 gate 不依赖 scheduler
    // ——依赖倒置在此一次性接上（此后 gate::snap() 即可取快照）。
    crate::work::unit::gate::install(crate::work::room::scheduler::core::roster);

    // 关机序列：messenger 簿记规模（仅 audit：只读观测，**必须在 rip 之前**——
    //   rip 清空站点表，之后再量恒为 0，那样的断言没有牙）→ scheduler::rip
    //   （清任务队列 + info 槽 + messenger 簿记）→ mail 由 drop 链透传
    //   （DockMeta::drop / RingMeta::drop）→ block 池冲洗 → audit 基线。
    #[cfg(feature = "audit")]
    const SHUTDOWN_HOOKS: &[fn()] = &[
        crate::memory::allocator::fence::audit::probe_messenger,
        crate::work::room::scheduler::core::rip,
        crate::memory::allocator::block::flush,
        crate::memory::allocator::fence::audit::check_baseline,
    ];
    #[cfg(not(feature = "audit"))]
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
/// 之后所有任务都由 root 产生（`Build`/`Spawn`/`Hatch`）；系统在全部任务回收后
/// 自然停机（`conductor::done`）。清单与设备语义的**解释权都在 root**——内核不含
/// 清单格式，也不解释设备（`docs/driver.md` §3.1.3）。
fn spawn_root() -> Result<(), MapError> {
    let Some(region) = machine::info().initrd() else {
        return Ok(());
    };
    // initrd 区恒等映射（=物理地址），直接按其物理基址读。
    let blob: &'static [u8] =
        unsafe { core::slice::from_raw_parts(region.base as *const u8, region.size) };
    let elf = crate::initrd::root_image(blob).expect("initrd: root image missing");

    let name = env::Name::new("root").expect("root name");
    let team = crate::work::unit::build(
        elf,
        crate::work::unit::space::SpaceKind::Supervisor,
        name,
        alloc::sync::Weak::new(),
    )
    .expect("assemble root elf");

    // 清单视图：在 root 的用户段里**登记**一段 VA（lowest first-fit，紧接镜像），
    // 把 initrd 区（持久保留区，帧分配器永不动它）只读借用映射进去。VA 与长度
    // 经启动参数告知 root——内核不含清单格式。
    let view_size = region.size.next_multiple_of(PAGE_SIZE);
    let view = team.space.with_flush(
        |inner| -> Result<crate::memory::manager::addr::VirtAddr, MapError> {
            let va = inner.allocate(crate::work::unit::space::SegmentKind::NonKernel, view_size)?;
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
    let devices = crate::devices::scan();
    let (pairs_pa, pairs_bytes) = crate::devices::block();
    let pairs = team.space.with_flush(
        |inner| -> Result<crate::memory::manager::addr::VirtAddr, MapError> {
            let va = inner.allocate(
                crate::work::unit::space::SegmentKind::NonKernel,
                pairs_bytes,
            )?;
            inner.borrow(
                va,
                crate::memory::manager::addr::PhysAddr::from_raw(pairs_pa),
                pairs_bytes,
                read_only(),
            )?;
            Ok(va)
        },
    )?;

    // 引导线程：args = [清单视图 VA, 清单字节数, 配对块 VA, 设备条数]；boot 立即放行。
    let bootstrap = team
        .task()
        .name("bootstrap")
        .args(vec![
            view.as_usize(),
            region.size,
            pairs.as_usize(),
            devices.len(),
        ])
        .spawn()?;
    crate::devices::install(&bootstrap, devices);

    #[cfg(feature = "audit")]
    team.space.audit();

    #[cfg(feature = "audit")]
    kernel().expect("kernel team not initialized").space.audit();

    Ok(())
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
    let me = machine::hart_id();
    machine::mark_hart_started(me);
    let count = machine::hart_count();
    let entry = core::ptr::addr_of!(_boot_entry) as usize;
    for hart in 0..count {
        if hart == me {
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
        machine::mark_hart_started(hart);
    }
}

/// 副核主流程：per-hart CSR 配置后进入 idle。
#[unsafe(no_mangle)]
pub(crate) extern "C" fn boot_main() -> ! {
    // 副核 per-hart 初始化：先取共享内核 token（从**本 hart** 帧读——所有先上
    // 台的核都在 trap::init 填过相同的 kernel_satp，读自身帧语义最贴 per-hart；
    // 其余 per-hart CSR（stvec/sscratch/sie）与 hart 0 走同一原语 trap::arm_hart。
    let me = machine::hart_id();
    let ktc = kernel()
        .expect("kernel team not initialized")
        .space
        .translate(machine::hart_frame())
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
    // 登记内核驻留（ASID 0）：本核刚 `sfence.vma` 过。副核在此之前是 VACANT 态
    // （PerHart 静态初值），不会被任何清退选中。
    crate::memory::manager::asid::set_asid(crate::memory::manager::asid::Asid::kernel());
    // 启动完成写进 trace（直打控制台会扰 panic 现场）。
    trace::note(trace::EventKind::Boot(trace::BootEvent::Done { hart: me }));
    scheduler::boot::idle()
}
