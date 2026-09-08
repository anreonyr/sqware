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
use crate::work::mail::HoleMeta;
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
            ("hart count", format!("{} H", m.hart)),
            ("hart this", format!("{}", machine::hart_id())),
            ("timebase", format!("{} Hz", m.hertz)),
            (
                "dram",
                format!("{:#x}..{:#x}", m.dram.base, m.dram.range().end),
            ),
            (
                "free",
                format!("{:#x}..{:#x}", m.free.base, m.free.range().end),
            ),
            ("uart", format!("{:#x}", m.uart.base)),
            ("plic", format!("{:#x}", m.plic.base)),
            ("clint", format!("{:#x}", m.clint.base)),
            ("trap vector", format!("{:#x}", alltraps_va())),
            (
                "kernel frames",
                format!(
                    "{:#x}..{:#x}",
                    HART_FRAME_BASE.as_usize(),
                    HART_FRAME_BASE.as_usize() + m.hart * PAGE_SIZE
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

    // 健康检查（spare 预算验收恒跑 + PT 回收自测 debug）：任一失败
    // fail-fast（panic → crash scene）。
    crate::health::run();

    spawn_demos().expect("boot spawn failed");

    // boot 装配收尾（push 通道关门）：标记 `BOOT_DONE` 让 `done()` 守门放
    // 行——防 PUSHED==0（0 任务）永久误判为"全部结束"，系统永远停不了机。
    // **必须在 HSM 拉起副核之前**——副核从 idle() 进 run()/wait() 读 done()
    // 时见到 true，则 PUSHED==0 立即 halt；否则一直 WFI 等不会到达的 IPI。
    crate::work::room::conductor::boot_done();

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

    // 每条 reaped 任务：doom 级联（父删子随）——读该 task 的 heir → cull 子域。
    // mail 资源释放走 Task::drop 链透传，无需 task_exit。
    static EXIT_HOOKS: &[fn(usize)] = &[crate::work::room::messenger::doom];
    messenger::register_exit_hooks(EXIT_HOOKS);

    // 关机序列：scheduler::rip（清任务队列 + info 槽 + messenger 簿记）→
    //   mail 由 drop 链透传（DockMeta::drop / RingMeta::drop）→ block 池冲洗 → audit
    #[cfg(feature = "audit")]
    const SHUTDOWN_HOOKS: &[fn()] = &[
        crate::work::room::scheduler::core::rip,
        crate::memory::allocator::block::flush,
        crate::memory::allocator::fence::audit::check_baseline,
    ];
    #[cfg(not(feature = "audit"))]
    const SHUTDOWN_HOOKS: &[fn()] = &[
        crate::work::room::scheduler::core::rip,
        crate::memory::allocator::block::flush,
    ];
    conductor::register_shutdown_hooks(SHUTDOWN_HOOKS);
}

/// 清单 kind → 空间 kind：映射放适配层（`initrd` 是引导供给层，不反向依赖 `work::unit`）。
impl From<crate::initrd::ProgramKind> for crate::work::unit::space::SpaceKind {
    fn from(k: crate::initrd::ProgramKind) -> Self {
        match k {
            crate::initrd::ProgramKind::User => Self::User,
            crate::initrd::ProgramKind::Supervisor => Self::Supervisor,
        }
    }
}

/// 生成全部启动任务：initrd 小清单按名取程序（见 [`crate::initrd`]）——shell 装成
/// U 态团队，echo 装成 supervisor 域（S 态页表 + 独立 ASID）。错误统一 `?` 上抛。
fn spawn_demos() -> Result<(), MapError> {
    use crate::work::mail::hole;
    use crate::work::unit::gate::{self, AnyPie, Permission};

    // 读 initrd 字节来源（QEMU `-initrd` 经 `/chosen` 暴露；无配置 → 无程序）。
    // initrd 区恒等映射（=物理地址），直接按其物理基址读。
    let blob: &'static [u8] = match machine::info().initrd {
        Some(r) => unsafe { core::slice::from_raw_parts(r.base as *const u8, r.size) },
        None => &[],
    };
    if blob.is_empty() {
        return Ok(());
    }
    let programs = crate::initrd::programs(blob).expect("initrd: malformed manifest");

    // 两个引导域：shell（U 态页表）与 echo（S 态 supervisor 域）。装成哪种空间由
    // 清单携带（`ProgramKind`）——boot 不再硬编码特权级。
    let shell = crate::initrd::take(&programs, "shell");
    let (shell_team, shell_entry) =
        crate::work::unit::assemble(shell.elf, alloc::sync::Weak::new(), shell.kind.into())
            .expect("assemble shell elf");
    let echo = crate::initrd::take(&programs, "echo");
    let (echo_team, echo_entry) =
        crate::work::unit::assemble(echo.elf, alloc::sync::Weak::new(), echo.kind.into())
            .expect("assemble echo elf");

    // 目录入口门闩：内核是根授予的源头 → 原始自持（vestor = None）。它没有 envcall
    // 入口（class 7 已删除）：内核把这一份放进首个用户任务的权限表，靠 `Collect` 取。
    let (dreq, dreq_id) = hole::meta().map_err(|_| MapError::OutOfMemory)?;
    let dir_entry = gate::new_pie(
        dreq_id,
        Permission::READ | Permission::WRITE | Permission::VEST,
        None,
        alloc::sync::Arc::downgrade(&dreq),
    );

    // 回信 hole（目录 → 主 client）。v1 单 client：内核预置这条通道并把它交进
    // 调用方权限表（索引 1），故无需向用户态传任何整数。
    let (reply, reply_id) = hole::meta().map_err(|_| MapError::OutOfMemory)?;
    let reply_pie = gate::new_pie(
        reply_id,
        Permission::READ | Permission::WRITE,
        None,
        alloc::sync::Arc::downgrade(&reply),
    );

    // echo 入口 hole：域侧一枚（pull 请求）+ 目录侧一枚（Connect 转授）。
    let (eentry, eentry_id) = hole::meta().map_err(|_| MapError::OutOfMemory)?;

    // shell 先建：它的 task id 就是目录认定的 caller（身份由内核给，不走消息体）。
    let shell_id = shell_team.task().name("shell").entry(shell_entry).spawn()?;
    // echo 域任务：跑在 supervisor 空间上（SPP=1、独立 ASID）。
    let echo_id = echo_team.task().name("echo-svc").entry(echo_entry).spawn()?;

    // 服务系统：目录（内核闭包任务）+ 绑定 echo 入口门闩。
    spawn_services(dreq, reply, shell_id, eentry.clone(), eentry_id, echo_id)?;

    // 根授予（boot 期无任务在跑，副核未拉起、hart 0 未进调度，故无竞态）：
    //   shell 权限表 [0] 目录入口门闩、[1] 回信 pie；
    //   echo 域权限表 [0] 自己的入口门闩（pull 请求用）。
    {
        let shell = task_by_id(shell_id);
        let mut pies = shell.pies.lock();
        pies.push(AnyPie::Hole(dir_entry));
        pies.push(AnyPie::Hole(reply_pie));
    }
    {
        let mine = gate::new_pie(
            eentry_id,
            Permission::READ | Permission::WRITE,
            None,
            alloc::sync::Arc::downgrade(&eentry),
        );
        let echo = task_by_id(echo_id);
        echo.pies.lock().push(AnyPie::Hole(mine));
    }

    #[cfg(feature = "audit")]
    shell_team.space.audit();
    #[cfg(feature = "audit")]
    echo_team.space.audit();

    #[cfg(feature = "audit")]
    kernel().expect("kernel team not initialized").space.audit();

    Ok(())
}

/// 按 task id 取强引用（boot 期任务已登记，取不到即引导错误）。
fn task_by_id(id: usize) -> alloc::sync::Arc<crate::work::unit::task::Task> {
    crate::work::room::scheduler::core::lookup_task_by_id_weak(id)
        .and_then(|w| w.upgrade())
        .expect("boot task registered")
}

// ── 服务系统：目录（dispatcher）+ 已注册服务（echo） ──

/// 生成服务目录（内核闭包任务）并绑定 echo 入口门闩；返目录 task id。
///
/// 目录只有**一个 req hole**：回信走内核预置的通道（`reply`），调用方身份由内核
/// 给出（`caller`）。授权一律走 `gate::accord`，目录不跨任务写调用方权限表。
/// 服务本体不在此处——echo 跑在独立 supervisor 域里（`spawn_demos` 装载）。
fn spawn_services(
    dreq: alloc::sync::Arc<HoleMeta>,
    reply: alloc::sync::Arc<HoleMeta>,
    caller: usize,
    eentry: alloc::sync::Arc<HoleMeta>,
    eentry_id: crate::work::mail::ResourceId,
    echo_id: usize,
) -> Result<usize, MapError> {
    use crate::service::dispatch;
    use crate::work::unit::gate::{self, Permission};
    use env::dispatch::Name;

    let kt = kernel().expect("kernel team not initialized");
    let registry = dispatch::new_registry();

    // 目录 task（独占持 registry Arc——闭包生命周期 = registry 生命周期）
    let dreq_svc = dreq.clone();
    let reply_svc = reply.clone();
    let registry_svc = registry.clone();
    let dir_id = kt
        .task()
        .name("dispatcher")
        .closure(move || {
            #[inline(never)]
            fn svc(
                dreq: alloc::sync::Arc<HoleMeta>,
                reply: alloc::sync::Arc<HoleMeta>,
                caller: usize,
                reg: alloc::sync::Arc<dispatch::ServiceRegistry>,
            ) -> ! {
                use crate::work::mail::hole;
                use crate::work::room::messenger::WaitKey;
                loop {
                    let pull_k = hole::pull_key(&dreq);
                    crate::work::room::scheduler::ktask::wait_mail(WaitKey::into_raw(pull_k));
                    let Ok(msg) = hole::pull(&dreq) else { continue };
                    let Some(me) = crate::work::room::scheduler::core::current().running_task()
                    else {
                        continue;
                    };
                    let out =
                        crate::service::dispatch::serve(&reg, &me, caller, &msg).encode();
                    while hole::push(&reply, &out).is_err() {
                        let push_k = hole::push_key(&reply);
                        crate::work::room::scheduler::ktask::wait_mail(WaitKey::into_raw(push_k));
                    }
                }
            }
            svc(dreq_svc, reply_svc, caller, registry_svc)
        })?;

    // 绑定 echo：入口门闩 vestor = echo task id（owner），故只有 echo 能解绑/换绑。
    // wire 上的 Register/Unregister/Replace 留给用户态服务。
    let entry_pie = gate::new_pie(
        eentry_id,
        Permission::READ | Permission::WRITE | Permission::VEST,
        Some(echo_id),
        alloc::sync::Arc::downgrade(&eentry),
    );
    dispatch::bind(
        &registry,
        Name::new("echo").expect("valid service name"),
        &entry_pie,
    )
    .expect("bind echo");

    Ok(dir_id)
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
            .args(sbi::scall::SArgs {
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
