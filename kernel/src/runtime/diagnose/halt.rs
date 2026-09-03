// 内核 panic 处理器（halt）— 组成诊断报告后停机。
//
// 多核 panic 策略：只保留报警源（第一个 panic 的）hart，其余 hart 经 `hush()` →
// `hunker()` 就地卧倒（关中断 + HSM 自停）；胜出的 ALARMER 经 `home()` 归巢——
// 落盘 SCENE、切 ROOT 栈、组稿（组稿不再依赖可能已损坏的触发栈）。
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::layout::ROOT_STACK_SIZE;
use crate::machine;
use crate::runtime::diagnose::report::Report;
use sbi::{self, fid, scall::SArgs};

/// 警报是否已拉响（第一个 panic 置位；其余 hart 依此停止）。
/// 与 follower 的 Acquire 读配对：`ALARM` 置位可见时，`ALARMER` 的写入亦必可见。
static ALARM: AtomicBool = AtomicBool::new(false);
/// 报警源（正在 handle panic 的）hart id；哨兵 `usize::MAX` = 尚未记录。
/// 仅报警源在抢占成功后写入（follower 不写），故恒为真正报警源的 id。
static ALARMER: AtomicUsize = AtomicUsize::new(usize::MAX);

/// 非报警源核「噤声」：就地卧倒（不返回）。正常运行时恒 no-op（一次 Acquire 读 + 分支）。
pub fn hush() {
    if ALARM.load(Ordering::Acquire) && ALARMER.load(Ordering::Acquire) != machine::hart_id() {
        hunker();
    }
}

/// 就地卧倒：关中断 + HSM 自停（真正离线）；HSM 不可用则关中断自旋兜底。
/// 两条路径都不再触碰任何业务内存（无分配、无锁、无打印），且永不返回。
fn hunker() -> ! {
    // 关全局中断：内核态本就 SIE=0（恒关中断策略），此处防御性再清一次，
    // 确保 HSM 前的瞬间没有中断被取进来（防「刚停止又被唤醒」的竞态）。
    // SAFETY: 仅清 sstatus.SIE 位，纯写本 hart 自己的 CSR，无并发别名。
    unsafe { core::arch::asm!("csrci sstatus, 2") };
    // 尽力 HSM 自停：成功后本 hart 停在被唤醒前的状态，不再执行任何指令。
    let _ = sbi::HsmCall::new(fid::Hsm::Stop).call();
    // 兜底（HSM 不可用 / 调用返回）：关中断自旋——同样不再触碰业务内存。
    loop {
        core::hint::spin_loop();
    }
}

/// 抢占为报警源：返回 true = 本 hart 首次胜出（继续转储）；false = 本 hart **已
/// 是**报警源（嵌套 panic re-entry——不再重复转储，调用方直接进停机自环）。
///
/// 原子互斥保证并发 panic 只有一个报警源：CAS 失手即输——输家若**不是**报警源
/// 就地 `hunker` 卧倒；输家若**就是**报警源（现场转储过程中再 fault 的嵌套路径）
/// 则**不卧倒**。`ALARMER` 仅在胜出后写入并 Release 发布。
fn claim() -> bool {
    if ALARM.swap(true, Ordering::AcqRel) {
        // 已有人报警：唯一例外是本核自己就是报警源（嵌套 panic）——豁免卧倒。
        if ALARMER.load(Ordering::Acquire) != machine::hart_id() {
            hunker();
        }
        return false;
    }
    ALARMER.store(machine::hart_id(), Ordering::Release);
    true
}

/// 向所有其它已启动 hart 广播 SBI IPI（错误尽力而为）。按 64 核一组循环
/// （SBI `sbi_send_ipi` 掩码至多 XLEN=64 位）；排除自身。
fn broadcast() {
    let me = machine::hart_id();
    let n = machine::hart_count();
    for w in 0..n.div_ceil(usize::BITS as usize) {
        let base = w * (usize::BITS as usize);
        let hi = (base + (usize::BITS as usize)).min(n).saturating_sub(base);
        let mut mask = 0usize;
        for b in 0..hi {
            let hart = base + b;
            if hart != me {
                mask |= 1usize << b;
            }
        }
        if mask == 0 {
            continue;
        }
        let _ = sbi::IpiCall::new(fid::Ipi::SendIpi)
            .args(SArgs {
                a0: mask,
                a1: base,
                ..Default::default()
            })
            .call();
    }
}

/// 拉响警报：抢占为报警源（并发 panic 的输家在此卧倒，不返回），随后广播唤醒
/// 及其它核。返回 true = 本 hart 首次胜出（可继续打印/停机）；false = 嵌套
/// re-entry（本 hart 已是报警源，报警/广播已做过，调用方直接进停机自环）。
fn alarm() -> bool {
    let won = claim();
    if won {
        broadcast();
    }
    won
}

/// 归巢落盘的原始现场（`home` 写 [sp, fp]；`(0, 0)` = 未归巢——`crash_scene!`
/// 直调无现场，kbt 回落读当前 sp/fp）。写 = ALARMER（同 hart 程序序）；asm 写
/// 入对编译器不透明，读侧 volatile。
static mut SCENE: [usize; 2] = [0; 2];

/// 原始现场读取器（kbt 溯源）：`home` 落盘值；`(0, 0)` = 未归巢。
pub(crate) fn scene() -> (usize, usize) {
    // SAFETY: 单写单读同 hart（ALARMER 归巢路径）；volatile 防 asm 写入被缓存。
    let s = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(SCENE)) };
    (s[0], s[1])
}

#[panic_handler]
pub(crate) fn panic_handler(info: &PanicInfo) -> ! {
    // 拉响警报：抢占报警源（输家就地卧倒），再广播停止其它核，随后归巢组稿。
    // 嵌套 panic（重入）= `claim()` 假分支，恒为「本核即报警源」——直接进停机自环。
    // 仲裁**先行**于换栈：双核同时 panic 若先换栈，输家会与胜家争夺同一 ROOT 栈顶。
    if !alarm() {
        crate::putln!(
            "info: {} sepc={:#x} stval={:#x}",
            info.message(),
            riscv::register::sepc::read(),
            riscv::register::stval::read(),
        );
        halt_loop()
    }
    home(info)
}

/// 归巢（naked）：落盘 SCENE（原始 sp/fp）→ 跳 ROOT 栈顶 → `tail` panic_work。
/// 启动期 panic / 已归巢重入兜底：当前 sp 已在 ROOT 区间则不切。
/// a0 = &PanicInfo（panic_handler 原样传入；本函数不触碰 a0，tail 移交 panic_work）。
#[allow(improper_ctypes_definitions)]
#[unsafe(naked)]
extern "C" fn home(_info: &PanicInfo) -> ! {
    core::arch::naked_asm!(
        // SCENE：原始 sp/fp 落盘（kbt 溯源；写 = ALARMER，同 hart 程序序）
        "la   t0, {scene}",
        "sd   sp, 0(t0)",
        "sd   s0, 8(t0)",
        // ROOT 栈顶 = _kernel_edge + ROOT_STACK_SIZE（链接符号，恒等寻址）
        "la   t0, _kernel_edge",
        "li   t1, {size}",
        "add  t1, t0, t1",
        // 当前 sp 已在 [edge, edge+size)（启动期 panic / 已归巢重入）→ 不切
        "mv   t2, sp",
        "bltu t2, t0, 1f",
        "bltu t2, t1, 2f",
        "1:  mv   sp, t1",
        "2:  tail {work}",
        scene = sym SCENE,
        size = const ROOT_STACK_SIZE,
        work = sym info,
    );
}

/// 归巢组稿：在 ROOT 栈上完成诊断报告、崩溃现场转储与停机。
#[allow(improper_ctypes_definitions)]
extern "C" fn info(info: &PanicInfo) -> ! {
    // 归巢审核：ROOT 栈底 canary 复读——boot 移交后 ROOT 应无人使用；此处捕
    // 「boot 后 ROOT 被误用 / boot 期溢出未捕」类内核 bug。报告一行，不递归。
    // SAFETY: canary 由 `_start` 写入、boot 移交审核过；此处 volatile 复读。
    let root_ok = unsafe { (crate::machine::root_stack_base() as *const usize).read_volatile() }
        == crate::machine::ROOT_STACK_CANARY;

    // 门户后端无锁切到后备仓（spare）：不取锁，即使 panic 恰在持主堆锁现场也绝不卡死。
    crate::memory::allocator::portal::switch(crate::memory::allocator::portal::Backend::Spare);

    let mut report = Report::default();
    {
        let mut head = String::from("[panic]");
        if let Some(loc) = info.location() {
            head.push_str(&format!(
                " at {}:{}:{}",
                loc.file(),
                loc.line(),
                loc.column()
            ));
        }
        let mut rows: Vec<Vec<Option<String>>> = Vec::new();
        // 身份槽读一次（无锁）：Live = 正在跑的任务；Last = 末次任务（idle 核崩溃）。
        if let Some(i) = crate::work::room::scheduler::core::ident() {
            rows.push(vec![Some(format!(
                "task #{} '{}' @ hart {}",
                i.id(),
                i.name(),
                machine::hart_id()
            ))]);
        }
        rows.push(vec![Some(format!("{}", info.message()))]);
        rows.push(vec![Some(format!(
            "root stack @ {} : {}",
            if root_ok { "ok" } else { "CORRUPTED" },
            machine::hart_id()
        ))]);
        report.paragraph("panic", Some(head)).items.extend(rows);
    }

    crate::runtime::diagnose::trace::note(crate::runtime::diagnose::trace::EventKind::Halt(
        crate::runtime::diagnose::trace::HaltEvent::Panic,
    ));
    crate::runtime::diagnose::scene::dump_crash(&mut report);

    let sealed = report.seal();
    crate::putln!();
    let mut sink = crate::console::Sink;
    crate::runtime::diagnose::render::render(sealed, &mut sink, 2);
    #[cfg(feature = "semihosting")]
    crate::runtime::diagnose::export::export(sealed);

    halt_loop()
}

/// 停机自环：srst 关机/复位，失败则关中断 wfi 兜底自旋。**不 panic**（已是 panic 末端）。
fn halt_loop() -> ! {
    // SAFETY: 仅清 sstatus.SIE（纯写本 hart 自己的 CSR，与 hunker 同契约）。
    unsafe { core::arch::asm!("csrci sstatus, 2") };
    loop {
        let _ = sbi::SystemResetCall::new(fid::SystemReset::SystemReset).call();
        unsafe { core::arch::asm!("wfi") };
    }
}
