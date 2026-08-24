// 内核 panic 处理器（halt）— 组成诊断报告后停机
//
// 多核 panic 策略：**只保留报警源（第一个 panic 的）hart，其余 hart 停止**。
//   - `alarm()`：抢占成为报警源（原子互斥，输家即 `hunker()`），置 `ALARMER`，
//     广播 SBI IPI 唤醒/提示所有其它核，然后组成诊断报告，停机自环。
//   - 其余 hart（follower，含并发 panic 者）经 `hush()`（trap 入口 / 调度
//     `run()` 循环钩子）检测到警报后 `hunker()` 就地卧倒——关中断 + HSM 自停
//     （真正离线，不再执行任何指令），彻底不再触碰内存。
//
// 命名隐喻（警报族，全单字无下划线）：`alarm` 拉响警报 → 各核 `hush` 噤声
// （非报警源即 `hunker` 卧倒）→ 只剩 `ALARMER` 那一个核在继续。
// panic 路径故意绕过所有锁：经 `console::_write` 无锁直写控制台。
//
// 组稿形态（d8 设计的落地）：头部文本（[panic] 位置/任务/消息）作为 **panic
// 段**（单槽行 = 文本行）进报告，随后 scene/trace 段由 dump_crash 投稿；成册
// 打戳后一次印发——旧收集器块（depend 报警源）先行，再印新报告，最后宿主
// 导出整档快照。全部信息一个出口、一个顺序。
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::machine;
use crate::runtime::diagnose::report::Report;
use sbi::{self, fid, scall::SArgs};

/// 警报是否已拉响（第一个 panic 置位；其余 hart 依此停止）。
/// 与 follower 的 Acquire 读配对：`ALARM` 置位可见时，`ALARMER` 的写入亦必可见。
static ALARM: AtomicBool = AtomicBool::new(false);
/// 报警源（正在 handle panic 的）hart id；哨兵 `usize::MAX` = 尚未记录。
/// 仅报警源在抢占成功后写入（follower 不写），故恒为真正报警源的 id。
static ALARMER: AtomicUsize = AtomicUsize::new(usize::MAX);

/// 非报警源核“噤声”：系统已进入警报且本 hart 非报警源 → 就地卧倒（不返回）。
///
/// 供 trap 入口与调度器 `run()` 循环钩子调用；正常运行时恒 no-op（一次
/// Acquire 读 + 分支）。卧倒路径 `hunker` 不依赖任何锁/分配器。
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
/// 就地 `hunker` 卧倒；输家若**就是**报警源（`ALARMER == me`，即现场转储过程中
/// 再 fault 的嵌套路径）则**不卧倒**——卧倒会把本 hart HSM 停掉，srst 停机自环
/// 永远到不了（"panic 后不停机"的根路径）。`ALARMER` 仅在胜出后写入并 Release
/// 发布——窗口期读到的哨兵 `usize::MAX` 使任何非报警核都判为 follower→`hunker`。
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

/// 向所有其它已启动 hart 广播 SBI IPI（唤醒睡核、提示用户核）；错误尽力而为。
///
/// 按 64 核一组循环（SBI `sbi_send_ipi` 掩码至多 XLEN=64 位，超 64 需递增
/// mask_base）；排除自身（报警源在停机自环，不卧倒自己）。
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

#[panic_handler]
pub(crate) fn panic_handler(info: &PanicInfo) -> ! {
    // 拉响警报：抢占报警源（唯一继续运行并打印的 hart，输家就地卧倒），
    // 再广播停止其它核（唤醒睡核、提示用户核），随后组成诊断报告。
    // 嵌套 panic（本 hart 已是报警源，组稿期间再 fault 的重入）：报警与
    // 广播已做过、其它核已停——不再重复组稿，直接进停机自环。
    // 幽灵指纹：重入绝不静默——把第二次 panic 的原话与现场 sepc/stval 直写
    // 控制台（PanicMessage 的 Display 与两个 usize 全经 format_args 落栈上
    // 缓冲，无堆无锁不碰分配器，崩溃路径纪律不破）。否则组稿期任意再 fault
    // 会连第二次 panic 的文本一起吞掉：控制台全静默却 exit 0（"幽灵 panic"）。
    // claim() 的 false 分支恒为「本核即报警源」的重入（输家已在内部 hunker）。
    if !alarm() {
        crate::putln!(
            "info: {} sepc={:#x} stval={:#x}",
            info.message(),
            riscv::register::sepc::read(),
            riscv::register::stval::read(),
        );
        halt_loop()
    }

    // 崩溃全局切换：门户后端**无锁**换到后备仓（spare）——此后所有 alloc（报
    // 告组稿/渲染/序列化）都进仓内，主堆锁链/完整性不再可信。
    // 原子 store 不取任何锁：即使 panic 恰在持主堆锁现场也绝不卡死。
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
        if let Some((tid, tname)) = crate::work::room::scheduler::running_task_info() {
            rows.push(vec![Some(format!(
                "running task #{tid} '{tname}' @ hart {}",
                machine::hart_id()
            ))]);
        }
        rows.push(vec![Some(format!("{}", info.message()))]);
        rows.push(vec![Some(format!(
            "other harts hushed only hart {} remains",
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

/// 停机自环：srst 关机/复位（QEMU 以 "QEMU: Terminated" 退出），失败则关中断
/// wfi 兜底自旋。**不 panic**：此处已是 panic 末端，再 panic 会经嵌套 re-entry
/// 回到本函数（listener 豁免已保证不会无限递归转储，但停机路径不引入新故障源）。
fn halt_loop() -> ! {
    // SAFETY: 仅清 sstatus.SIE（纯写本 hart 自己的 CSR，与 hunker 同契约）。
    unsafe { core::arch::asm!("csrci sstatus, 2") };
    loop {
        let _ = sbi::SystemResetCall::new(fid::SystemReset::SystemReset).call();
        unsafe { core::arch::asm!("wfi") };
    }
}

