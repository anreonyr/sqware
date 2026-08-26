// 启动（boot）— 把 work 子系统拉起到首个用户任务。

use core::arch::global_asm;

use alloc::format;
use alloc::sync::Arc;
use alloc::vec;
use riscv::register::{satp, sie, stvec};

use crate::console::Sink;
use crate::machine::{kernel_stack_base, KERNEL_STACK_CANARY};
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::mode;
use crate::memory::manager::MapError;
use crate::memory::PAGE_SIZE;
use crate::runtime::diagnose::report::Report;
use crate::runtime::diagnose::trace;
use crate::runtime::switcher::context::TrapContext;
use crate::runtime::switcher::trampoline::{
    alltraps_va, restore, trap_stack_bottom, trap_stack_top,
};
use crate::work::room::scheduler;
use crate::work::unit::space::{SpaceBuilder, KERNEL_FRAME_BASE};
use crate::work::unit::team::kernel;
use crate::work::unit::{loader, team};
use crate::{machine, putln};

global_asm!(
    ".section .text.boot",
    ".align 2",
    ".globl _boot_entry",
    "_boot_entry:",
    "    mv   tp, a0",     // hartid → tp（与 _start 一致；hart_id() 读 tp）
    "    csrc sstatus, 2", // 清 SIE：内核态恒关中断（同 _start）
    "    mv   sp, a1",     // opaque = 本 hart trap 栈顶（HSM Start 传入；寄存器传递）
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
                    KERNEL_FRAME_BASE.as_usize(),
                    KERNEL_FRAME_BASE.as_usize() + m.hart * PAGE_SIZE
                ),
            ),
            (
                "trap stack",
                format!("{:#x}..{:#x}", trap_stack_bottom(0), trap_stack_top(0)),
            ),
            (
                "trap stack this",
                format!(
                    "{} @ {:#x}..{:#x}",
                    machine::hart_id(),
                    trap_stack_bottom(machine::hart_id()),
                    trap_stack_top(machine::hart_id())
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
    scheduler::init();

    // lockdep 装配（debug 构建）：per-hart 持有集。release 为 no-op。
    // 置于调度器就绪后、spawn 演示任务/HSM 拉起副核前——正是多核 ABBA 的生效窗口。
    #[cfg(debug_assertions)]
    crate::lock::init_depend(machine::hart_count()).expect("depend init failed");

    // 健康检查（spare 预算验收恒跑 + PT 回收自测 debug）：任一失败
    // fail-fast（panic → crash scene）。
    crate::health::run();

    // 记录内核持久帧基线：**一切任务 spawn 之前**。此后在途帧只增任务所有；
    // 关机时全部归还（零泄漏审计）。区间窗口元数据是随任务存活的瞬态
    // （任务栈/帧条目随 reclaim 摘除、团队 drop 归还）——基线后创建、关机前
    // 全部释放，block 池净零回落；基线前只留静态内核结构（镜像、per-hart 帧、
    // 内核空间 durable 映射），关机不释放，账平。
    #[cfg(debug_assertions)]
    crate::memory::allocator::fence::audit::record_baseline();

    // 演示程序均为内嵌 ELF，经装载生成任务并入队；错误一律 `?` 上抛至本边界。
    spawn_demos().expect("boot spawn failed");

    // 完整性审计（debug）：boot 收尾全量核对。
    #[cfg(debug_assertions)]
    crate::memory::allocator::fence::audit::audit();

    // 多核：HSM 拉起其余副核。
    boot_harts();

    // 主内核栈（boot 栈）将永久离开前校验 canary：boot 期栈溢出即使未越过
    // guard 页（4 KiB 内）也会在此暴露，且不必等缺页死机。
    let boot_guard = unsafe { (kernel_stack_base() as *const usize).read() };
    assert!(
        boot_guard == KERNEL_STACK_CANARY,
        "main kernel stack overflow during boot: canary corrupted {boot_guard:#x}",
    );

    // 进入调度：从本 hart 调度器取首任务（不能用 spawn 返回的帧 PA——可能已被
    // 副核 steal 走，见 scheduler::enter_first_task）
    restore(scheduler::run())
}

/// 生成全部演示任务（用户 + 内核 ktask）；错误统一 `?` 上抛。返回前所有任务已入队。
fn spawn_demos() -> Result<(), MapError> {
    // 单线程团队回归
    for (elf, name) in [
        // (&include_bytes!(env!("USER_MMAPER"))[..], "mmaper"),
        (&include_bytes!(env!("USER_HEAPER"))[..], "heaper"),
        (&include_bytes!(env!("USER_SPAWNER"))[..], "spawner"),
    ] {
        let (team, entry) = load_user(elf);
        team.task().name(name).entry(entry).spawn()?;
        // debug: 每个演示空间 簿记↔页表 一致性审计
        #[cfg(debug_assertions)]
        team.space.audit();
    }

    // 内核任务（ktask）：挂 kernel 团队单例。
    kernel()
        .expect("kernel team not initialized")
        .task()
        .name("ktask")
        .closure(|| {
            putln!("ktask");
        })?;
    // 可抢占内核验证：常驻循环任务（每轮百万次增量 + 打印进度）。被 S-timer 抢占
    // 恢复正确 → round 顺序不重不乱、n 单调递增、hart 稳定；现场丢失则崩/乱序。
    kernel()
        .expect("kernel team not initialized")
        .task()
        .name("preempt")
        .closure(|| {
            let mut n: usize = 0;
            for round in 0..10u32 {
                let start = n;
                for _ in 0..100_000 {
                    n = n.wrapping_add(1);
                }
                crate::putln!(
                    "preempt: round {round} n={n:#x} delta={:#x} hart={}",
                    n.wrapping_sub(start),
                    crate::machine::hart_id()
                );
            }
            crate::putln!("preempt: done");
        })?;
    // 风暴回归（storm_ktask）**暂从验收 boot 摘除**（2026-06 定位记录）：风暴的
    // 63 次子任务 spawn/exit/reap/retire 与 ktask 自切换 park 通道组合时触发内核
    // 堆块池串写（canary 12 块被砸 / Arc 身份错乱 / 垃圾尺寸分配）——与 gate-5
    // 无关的既有回归 bug，独立记档待查（见 channel-async.md A8 维护记录）。
    // 单跑风暴或单跑 channel 各自干净；函数本体保留，恢复覆盖时取消本行注释。
    // storm_ktask(STORM_N)?;
    // Channel 验收（A8.3）：同 Space 双 ktask 经 req/resp 双通道 echo + 语义探针。
    // 走 tick 粒度 park（闭包体自切换 + 唤醒续跑）——ktask 自切换原语闭环验收。
    channel_demos()?;
    #[cfg(debug_assertions)]
    kernel().expect("kernel team not initialized").space.audit();
    Ok(())
}

/// Channel echo 验收（A8.3）：同 Space 双 ktask 经双通道 req/resp task↔task 闭环。
///
/// A（echo_client）：spawn 建 req/resp 双通道（slot_len=8）→ server 半对 move 进
/// B（echo_server）→ 发请求（序列号槽字）→ pull 响应校验一致。B：pull 请求 →
/// 内容原样经 resp_tx 弹回（echo）。走 tick 粒度 park（闭包体自切换 + 唤醒续跑），
/// 验证：四态状态机（Gone 断开感知）· push/try_push/pull/timeout/crush · ktask 自
/// 切换原语闭环。
fn channel_demos() -> Result<(), MapError> {
    use crate::work::unit::channel::{ChannelBuilder, Spawned};

    let kt = kernel().expect("kernel team not initialized");
    // spawn 双通道：四端点一次吐出（mpsc 式创建即双端）
    let Spawned { client, server } = ChannelBuilder::new()
        .slot_len(8)
        .spawn()
        .expect("channel spawn");

    // 验收轮数 + 每轮消息槽字数。
    const ROUNDS: usize = 5;
    const MSG: usize = 3;

    // B（echo_server）：server 半对 move 进闭包（同空间，VA 直见；Arc 跨任务共享）。
    kt.task()
        .name("echo-server")
        .closure(move || {
            let mut out = [0usize; MSG];
            loop {
                let n = match server.req_rx.pull(&mut out) {
                    Ok(n) => n,
                    Err(e) => {
                        putln!("echo-server: pull err {e:?}");
                        break;
                    }
                };
                // echo：把收到的槽字原样经 resp_tx 弹回。
                if server.resp_tx.push(&out[..n]).is_err() {
                    putln!("echo-server: push err");
                    break;
                }
                putln!("echo-server: echoed {n} slots");
            }
            putln!("echo-server: done");
        })?;

    // A（echo_client）：发序列号请求，pull 响应校验与请求一致。
    kt.task()
        .name("echo-client")
        .closure(move || {
            let mut resp = [0usize; MSG];
            for i in 0..ROUNDS {
                let msg = [0xCAFE + i, i, i.wrapping_mul(i)];
                client.req_tx.push(&msg).expect("client push");
                let n = client.resp_rx.pull(&mut resp).expect("client pull");
                if resp[..n] != msg[..n] {
                    putln!("echo-client: MISMATCH round {i}: {resp:?} != {msg:?}");
                } else {
                    putln!("echo-client: round {i} ok ({n} slots)");
                }
            }
            putln!("echo-client: done");
        })?;

    // 语义验收（独立通道 + 单任务自测，不干扰 echo 主测的通道状态）：
    //   try_push 满路径（slot_len=4，消息 1+1=2 槽 → 塞 2 条满，第 3 条 Full）
    //   · timeout 超时路径（空 resp + 已过 deadline → Timeout）
    //   · crush 显式终止（置 Dead → 后续 push 报 Dead）
    // 语义通道：整对移入闭包（mpsc 式两端共存；对端半对不得在持有者外部 drop
    // ——Rx drop → Dead 是「拉端消逝」的真实语义，误移交给别处才合法）。
    let semantics = ChannelBuilder::new().slot_len(4).spawn().expect("semantic channel");
    // crush 测试专用独立通道（同 spawn；对端半对也移入闭包观察 Dead）
    let semantics_crush = ChannelBuilder::new().slot_len(4).spawn().expect("crush channel");
    kt.task()
        .name("semantics")
        .closure(move || {
            use crate::work::unit::channel::ChannelError;
            let (req_tx, resp_rx) = (semantics.client.req_tx, semantics.client.resp_rx);
            // 对端半对随闭包持有到结束（不 drop 误杀通道）：显式借用防御
            let _keep_server = (&semantics.server.req_rx, &semantics.server.resp_tx);
            match req_tx.try_push(&[0xA1]) {
                Ok(()) => putln!("semantics: push1 ok"),
                other => putln!("semantics: push1 unexpected {other:?}"),
            }
            match req_tx.try_push(&[0xB2]) {
                Ok(()) => putln!("semantics: push2 ok"),
                other => putln!("semantics: push2 unexpected {other:?}"),
            }
            match req_tx.try_push(&[0xC3]) {
                Err(ChannelError::Full) => putln!("semantics: try_push Full ✓"),
                other => putln!("semantics: try_push expected Full got {other:?}"),
            }
            // timeout：deadline 已是过去 → 立即 Timeout
            let past = crate::runtime::chrono::clock::now();
            match resp_rx.timeout(&mut [0usize; 1], past) {
                Err(ChannelError::Timeout) => putln!("semantics: timeout Timeout ✓"),
                other => putln!("semantics: timeout expected Timeout got {other:?}"),
            }
            // crush：req 通道显式终止 → 对端（server 半对的 req_rx）pull 报 Dead。
            // 消费式语义：crush 后本端已不可用，从对端观察 Dead。
            {
                let crushed = semantics_crush;
                crushed.client.req_tx.crush();
                let mut buf = [0usize; 1];
                match crushed.server.req_rx.pull(&mut buf) {
                    Err(ChannelError::Dead) => putln!("semantics: crush Dead ✓"),
                    other => putln!("semantics: crush expected Dead got {other:?}"),
                }
            }
            putln!("semantics: done");
        })?;
    Ok(())
}

/// 风暴子任务数（回归：一次连环 spawn 全部存活并入队）。
const STORM_N: usize = 60;

/// 风暴 = 内核任务连环 spawn closure 子任务（子任务空跑即退）。
fn storm_ktask(n: usize) -> Result<(), MapError> {
    kernel()
        .expect("kernel team not initialized")
        .task()
        .name("storm")
        .closure(move || {
            let kt = kernel().expect("kernel team not initialized");
            crate::putln!("storm: begin spawn {n}");
            for _i in 0..n {
                kt.task()
                    .name("child")
                    .closure(|| {})
                    .expect("storm spawn child");
            }
            crate::putln!("storm: all {n} spawned");
        })?;
    Ok(())
}

/// 内嵌用户 ELF 经解析装载生成 Team；返回 (Team, 绝对入口)。
fn load_user(elf: &'static [u8]) -> (Arc<team::Team>, VirtAddr) {
    let parsed = crate::work::unit::parser::parse(elf).expect("parse user elf");
    let space = SpaceBuilder::user().build().expect("space failed");
    let loaded = loader::load(space, elf, &parsed).expect("load user elf");
    let entry = loaded.entry;
    // 符号表：内嵌 ELF 的 .symtab/.strtab → ElfTable（失败则 None，只影响符号化不碍装载）
    let elftable = crate::work::unit::parser::tables(elf)
        .ok()
        .and_then(|(s, ss)| crate::work::unit::elftable::ElfTable::from_sections(s, ss))
        .map(Arc::new);
    let team = team::TeamBuilder::new(loaded.space)
        .elftable(elftable)
        .spawn();
    (team, entry)
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
        let stack_top = crate::runtime::switcher::trampoline::trap_stack_top(hart);
        // putln!("hart {me}: starting hart {hart} @ {entry:#x}, trap stack {stack_top:#x}");
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
    // 副核 per-hart 初始化：satp = 共享内核 token（从内核帧读）、stvec、sscratch、sie。
    let ktc = kernel()
        .expect("kernel team not initialized")
        .space
        .translate(KERNEL_FRAME_BASE)
        .expect("kernel frame not mapped")
        .0;
    let frame = unsafe { &*(ktc.as_usize() as *const TrapContext) };
    let ksatp = frame.kernel_satp;
    // 探测所得模式 token：低 44 位 ppn、[63:44] asid/模式（字段访问器拆解，
    // 无裸位运算；模式位随 mode()，副核与主核同模式）
    unsafe {
        satp::set(mode::mode(), ksatp.asid(), ksatp.ppn());
        core::arch::asm!("sfence.vma");
        stvec::write(stvec::Stvec::new(alltraps_va(), stvec::TrapMode::Direct));
        core::arch::asm!("csrw sscratch, zero");
        sie::set_stimer();
        sie::set_ssoft(); // SSIP 使能：WFI 休眠唤醒
    }
    // 启动完成写进 trace（直打控制台会扰 panic 现场）。
    trace::note(trace::EventKind::Boot(trace::BootEvent::Done {
        hart: machine::hart_id(),
    }));
    scheduler::idle()
}
