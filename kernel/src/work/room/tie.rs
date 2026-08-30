// 系统级生命周期：跨核共享的原子状态与编排。
//
// 两件事：
//   全退出停机 — PUSHED/REAPED 任务计数；相等且 PUSHED>0 即全部退出 → 发 SBI srst 复位；
//              HALTING 做一次性互斥，防多核同时发复位。
//   休眠唤醒   — WAITING 位图（bit h = hart h 正 WFI 等待）；入队后 yell
//              按掩码一次 SBI IPI 喊醒全部睡核。
//
// 命名：动词（spawn/exit/done/halt/sleep/wake/yell/wfi）+ 计数名词
// （PUSHED/REAPED/WAITING/HALTING）。

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::machine;
use crate::putln;
use sbi::scall::SArgs;
use sbi::{self, fid};

/// 已入队（创建）任务计数（全退出检测：REAPED == PUSHED → 停机）。
static PUSHED: AtomicUsize = AtomicUsize::new(0);
static REAPED: AtomicUsize = AtomicUsize::new(0);
/// 停机互斥：第一个触发 srst 的核胜出，其余 wfi（避免双 srst）。
static HALTING: AtomicBool = AtomicBool::new(false);
/// 已到达 halt 的核数 — 关机屏障：胜出核须等**全部**核到达后再断言帧基线。
static HALT_ARRIVED: AtomicUsize = AtomicUsize::new(0);
/// WFI 等待唤醒的 hart 位图（字 w 的 bit b = hart `w·64 + b` 正阻塞）。
/// 多字：字宽 = SBI 单次 `sbi_send_ipi` 的寻址窗口（XLEN = 64 位），按
/// MAX_HART_SLOTS 分字。位位置 = hartid。
static WAITING: [AtomicUsize; WAITING_WORDS] = [const { AtomicUsize::new(0) }; WAITING_WORDS];

/// WAITING 位图字数：每字 64 位（= 协议单次 IPI 掩码窗口）。
const WAITING_WORDS: usize = crate::machine::MAX_HART_SLOTS / usize::BITS as usize;

/// 任务入队计数 +1（PUSHED）。Relaxed 够用：计数只用于相等比较，且自增发生在持调度锁时。
pub(super) fn push() {
    PUSHED.fetch_add(1, Ordering::Relaxed);
}

/// 任务回收计数 +1（REAPED）。
pub(super) fn exit() {
    REAPED.fetch_add(1, Ordering::Relaxed);
}

/// 全部任务是否已退出（PUSHED > 0 防 boot 早期误判）。
pub(super) fn done() -> bool {
    let pushed = PUSHED.load(Ordering::Relaxed);
    pushed > 0 && REAPED.load(Ordering::Relaxed) == pushed
}

/// 全部任务已退出：显式停机（srst；AtomicBool 防双核同时触发——后到者 wfi）。
///
/// 关机屏障：胜出核等全部核到齐后冲洗块池、断言帧基线，最后复位。
pub(super) fn halt() -> ! {
    HALT_ARRIVED.fetch_add(1, Ordering::AcqRel);
    if !HALTING.swap(true, Ordering::AcqRel) {
        // 喊醒 WFI 睡核：它们醒来后同样会走 done → halt → 登记到达。
        yell();
        while HALT_ARRIVED.load(Ordering::Acquire) < machine::hart_count() {
            core::hint::spin_loop();
        }
        putln!("task: all tasks exited, system halted");
        crate::runtime::diagnose::trace::note(crate::runtime::diagnose::trace::EventKind::Halt(
            crate::runtime::diagnose::trace::HaltEvent::Halt,
        ));
        // 关机清理：dock/ring 注册表清空（全部任务已退、space 已 drop），触发
        // Meta drop 归还共享区帧——必须在帧基线审计前，否则残留 Arc 计入泄漏。
        crate::work::mail::dock::shutdown();
        crate::work::mail::ring::shutdown();
        crate::memory::allocator::block::flush();
        #[cfg(debug_assertions)]
        crate::memory::allocator::fence::audit::check_baseline();
        let _ = sbi::SystemResetCall::new(fid::SystemReset::SystemReset).call();
    }
    wfi()
}

/// WFI 自环直到系统复位（halt 两分支共用；复位由 SBI 重新拉起）。
fn wfi() -> ! {
    loop {
        unsafe { core::arch::asm!("wfi") };
    }
}

/// 标记 hart 进入 WFI 等待。调用方须在置位后**复查队列**再睡。
pub(super) fn sleep(hart: usize) {
    debug_assert!(
        hart < crate::machine::MAX_HART_SLOTS,
        "sleep hart {hart} beyond MAX_HART_SLOTS"
    );
    WAITING[hart / (usize::BITS as usize)]
        .fetch_or(1usize << (hart % (usize::BITS as usize)), Ordering::AcqRel);
}

/// 清除 hart 的等待标记（WFI 唤醒后 / 复查发现任务时调用）。
pub(super) fn wake(hart: usize) {
    debug_assert!(
        hart < crate::machine::MAX_HART_SLOTS,
        "wake hart {hart} beyond MAX_HART_SLOTS"
    );
    WAITING[hart / (usize::BITS as usize)].fetch_and(
        !(1usize << (hart % (usize::BITS as usize))),
        Ordering::AcqRel,
    );
}

/// 喊醒所有 WFI 等待中的 hart。按 64 核一组循环 SBI IPI（协议单次掩码至多 XLEN 位）。
pub(super) fn yell() {
    for (w, word) in WAITING.iter().enumerate() {
        let waiting = word.load(Ordering::Acquire);
        if waiting == 0 {
            continue;
        }
        let _ = sbi::IpiCall::new(fid::Ipi::SendIpi)
            .args(SArgs {
                a0: waiting,
                a1: w * (usize::BITS as usize),
                ..Default::default()
            })
            .call();
    }
}
