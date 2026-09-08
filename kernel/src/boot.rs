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
use crate::work::mail::HoleMeta;
use crate::work::mail::memo::ResourceId;
use crate::work::unit::team::kernel;
use crate::lock::OnceLock;

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

/// 生成全部启动任务：initrd 只承载 shell 一个 ELF，整块 blob 直接装载为 shell 域。
/// 错误统一 `?` 上抛。
fn spawn_demos() -> Result<(), MapError> {
    // 读 initrd 字节来源（QEMU `-initrd` 经 `/chosen` 暴露；无配置 → 无程序）。
    // initrd 区恒等映射（=物理地址），直接按其物理基址读。
    let blob: &'static [u8] = match machine::info().initrd {
        Some(r) => unsafe { core::slice::from_raw_parts(r.base as *const u8, r.size) },
        None => &[],
    };
    if blob.is_empty() {
        return Ok(());
    }

    // initrd = 单个 shell ELF，整块喂给 assemble。
    let (team, entry) =
        crate::work::unit::assemble(blob, alloc::sync::Weak::new()).expect("assemble shell elf");
    team.task().name("shell").entry(entry).spawn()?;
    #[cfg(feature = "audit")]
    team.space.audit();

    #[cfg(feature = "audit")]
    kernel().expect("kernel team not initialized").space.audit();

    // 服务系统：dispatcher 闭包独占持服务注册表 Arc；服务（echo）注册到该表。
    spawn_services()?;

    Ok(())
}

// ── 服务系统：dispatcher + 已注册服务（echo） ──

/// Dispatcher 的 req/rep hole（user → dispatcher / dispatcher → user）。
/// 与 echo 的 ECHO_REQ/REP 不同：dispatcher 走名字查找，echo 走服务数据通道。
pub(crate) static DISPATCHER_REQ: OnceLock<(ResourceId, alloc::sync::Arc<HoleMeta>)> = OnceLock::new();
pub(crate) static DISPATCHER_REP: OnceLock<(ResourceId, alloc::sync::Arc<HoleMeta>)> = OnceLock::new();

fn spawn_services() -> Result<(), MapError> {
    use crate::work::mail::hole;
    use crate::service::dispatch;
    let kt = kernel().expect("kernel team not initialized");

    // 1. dispatcher req/rep hole
    let (dreq, dreq_id) = hole::meta().map_err(|_| MapError::OutOfMemory)?;
    let (drep, drep_id) = hole::meta().map_err(|_| MapError::OutOfMemory)?;
    DISPATCHER_REQ.get_or_init(|| (dreq_id, dreq.clone()));
    DISPATCHER_REP.get_or_init(|| (drep_id, drep.clone()));

    // 2. echo svc req/rep hole
    let (ereq, ereq_id) = hole::meta().map_err(|_| MapError::OutOfMemory)?;
    let (erep, erep_id) = hole::meta().map_err(|_| MapError::OutOfMemory)?;

    // 3. 服务注册表：显式 Arc，clone 给 dispatcher 闭包捕获（独占持有）
    let registry = dispatch::new_registry();
    dispatch::register(
        &registry,
        "echo",
        ereq_id,
        alloc::sync::Arc::downgrade(&ereq),
        erep_id,
        alloc::sync::Arc::downgrade(&erep),
    );

    // 4. spawn echo svc（按需启用，wait_mail park）
    kt.task()
        .name("echo-svc")
        .closure(move || {
            #[inline(never)]
            fn svc(req: alloc::sync::Arc<HoleMeta>, rep: alloc::sync::Arc<HoleMeta>) -> ! {
                use crate::work::room::messenger::WaitKey;
                loop {
                    let pull_k = hole::pull_key(&req);
                    crate::work::room::scheduler::ktask::wait_mail(WaitKey::into_raw(pull_k));
                    let Ok(pulled) = hole::pull(&req) else { continue; };
                    let mut reply = pulled;
                    for b in reply.iter_mut() {
                        *b = b.wrapping_add(1);
                    }
                    while hole::push(&rep, &reply).is_err() {
                        let push_k = hole::push_key(&rep);
                        crate::work::room::scheduler::ktask::wait_mail(WaitKey::into_raw(push_k));
                    }
                }
            }
            svc(ereq, erep)
        })
        .map(|_| ())?;

    // 5. spawn dispatcher task（独占持 registry Arc——闭包生命周期 = registry 生命周期）
    kt.task()
        .name("dispatcher")
        .closure(move || {
            #[inline(never)]
            fn svc(
                dreq: alloc::sync::Arc<HoleMeta>,
                drep: alloc::sync::Arc<HoleMeta>,
                reg: alloc::sync::Arc<dispatch::ServiceRegistry>,
            ) -> ! {
                use crate::work::room::messenger::WaitKey;
                loop {
                    // park 直到 caller 推 lookup 请求
                    let pull_k = hole::pull_key(&dreq);
                    crate::work::room::scheduler::ktask::wait_mail(WaitKey::into_raw(pull_k));
                    let Ok(req_msg) = hole::pull(&dreq) else { continue; };
                    let sender_id = u64::from_le_bytes(
                        req_msg[0..8].try_into().unwrap_or([0u8; 8]),
                    ) as usize;

                    // 解析：[0..8] sender_task_id, [8..64] name bytes
                    let name_bytes = &req_msg[8..];
                    let name_len = name_bytes.iter().position(|&b| b == 0).unwrap_or(name_bytes.len());
                    let name = core::str::from_utf8(&name_bytes[..name_len]).unwrap_or("");

                    // 构造回复（默认全 0 = 未找到）
                    let mut reply = [0u8; crate::work::mail::HOLE_MSG_LEN];
                    if let Some(svcs) = dispatch::lookup(&reg, name) {
                        if let Some(svc) = svcs.into_iter().next() {
                            // 找 caller task，建 Pies 塞进 caller.pies
                            if let Some(sender_weak) =
                                crate::work::room::scheduler::core::lookup_task_by_id_weak(sender_id)
                            {
                                if let Some(sender) = sender_weak.upgrade() {
                                    let current_id = sender.ident.id;
                                    if let (Some(req_meta), Some(rep_meta)) =
                                        (svc.req.upgrade(), svc.rep.upgrade())
                                    {
                                            use crate::work::unit::gate::{new_pie, AnyPie, Permission};
                                        // caller push req / echo pull req 都需 READ|WRITE
                                        // caller pull rep / echo push rep 都需 READ|WRITE
                                        let req_pie: crate::work::unit::gate::Pie<HoleMeta> =
                                            new_pie(
                                                svc.req_id,
                                                Permission::READ | Permission::WRITE,
                                                Some(current_id),
                                                alloc::sync::Arc::downgrade(&req_meta),
                                            );
                                        let rep_pie: crate::work::unit::gate::Pie<HoleMeta> =
                                            new_pie(
                                                svc.rep_id,
                                                Permission::READ | Permission::WRITE,
                                                Some(current_id),
                                                alloc::sync::Arc::downgrade(&rep_meta),
                                            );
                                        let req_tk = req_pie.token();
                                        let rep_tk = rep_pie.token();
                                        reply[0..8].copy_from_slice(&req_tk.to_le_bytes());
                                        reply[8..16].copy_from_slice(&rep_tk.to_le_bytes());
                                        let mut pies = sender.pies.lock();
                                        pies.push(AnyPie::Hole(req_pie));
                                        pies.push(AnyPie::Hole(rep_pie));
                                    }
                                }
                            }
                        }
                    }

                    // 推回 reply（dispatcher → caller）
                    while hole::push(&drep, &reply).is_err() {
                        let push_k = hole::push_key(&drep);
                        crate::work::room::scheduler::ktask::wait_mail(WaitKey::into_raw(push_k));
                    }
                }
            }
            svc(dreq, drep, registry)
        })
        .map(|_| ())?;

    Ok(())
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
        // opaque = 该 hart trap 栈物理栈顶（装配产物块基址 + 布局常量段偏移组装）
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
    crate::memory::manager::asid::set_asid(0);
    // 启动完成写进 trace（直打控制台会扰 panic 现场）。
    trace::note(trace::EventKind::Boot(trace::BootEvent::Done { hart: me }));
    scheduler::boot::idle()
}
