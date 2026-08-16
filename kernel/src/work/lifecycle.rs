// 系统级生命周期：跨核共享的原子状态与编排（不属任何单核调度器）。
//
// 两件事：
//   全退出停机 — SPAWNED/EXITED 任务计数（入队 +1 / 退出 +1）；相等且 SPAWNED>0
//              （防 boot 早期误判）即全部退出 → halt_all 发 SBI srst 复位；
//              HALTING 做一次性互斥，防多核同时发复位（后到者 wfi 等复位）。
//   休眠唤醒   — SLEEPING 位图（bit h = hart h 正 WFI 休眠）；入队后 wake_sleepers
//              按掩码一次 SBI IPI 唤醒全部睡核（可来 steal）；置位/清位经
//              mark_sleeping/mark_awake（AcqRel），配合调用方的置位后复查闭合
//              「漏唤醒」窗口（见 scheduler::idle_wait）。

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::ecall::scall::SArgs;
use crate::ecall::{self, fid};
use crate::putln;

/// 已创建 / 已退出任务计数（全退出检测：EXITED == SPAWNED → 停机）。
static SPAWNED: AtomicUsize = AtomicUsize::new(0);
static EXITED: AtomicUsize = AtomicUsize::new(0);
/// 停机互斥：第一个触发 srst 的核胜出，其余 wfi（避免双 srst）。
static HALTING: AtomicBool = AtomicBool::new(false);
/// WFI 休眠 hart 位图（bit h = hart h 正阻塞在 WFI 等待唤醒；push 后据此
/// 发 IPI——休眠核醒来后可 steal 新任务）。
static SLEEPING: AtomicUsize = AtomicUsize::new(0);

/// 任务创建计数 +1（scheduler::push 收尾时调用）。
///
/// Relaxed 够用：计数只用于相等比较，且自增发生在持本核调度锁时。
pub(super) fn on_task_spawned() {
    SPAWNED.fetch_add(1, Ordering::Relaxed);
}

/// 任务退出计数 +1（scheduler::reap 清理后调用）。
pub(super) fn on_task_exited() {
    EXITED.fetch_add(1, Ordering::Relaxed);
}

/// 全部任务是否已退出（SPAWNED > 0 防 boot 早期误判）。
pub(super) fn all_exited() -> bool {
    let spawned = SPAWNED.load(Ordering::Relaxed);
    spawned > 0 && EXITED.load(Ordering::Relaxed) == spawned
}

/// 全部任务已退出：显式停机（srst；AtomicBool 防双核同时触发——后到者 wfi）。
pub(super) fn halt_all() -> ! {
    if !HALTING.swap(true, Ordering::AcqRel) {
        putln!("task: all tasks exited, system halted");
        let _ = ecall::SystemResetCall::new(fid::SystemReset::SystemReset).call();
    }
    wfi_forever()
}

/// WFI 自环直到系统复位（halt_all 两分支共用；复位由 SBI 重新拉起）。
fn wfi_forever() -> ! {
    loop {
        unsafe { core::arch::asm!("wfi") };
    }
}

/// 标记 hart 进入 WFI 休眠。调用方须在置位后**复查队列**再睡：置位前的
/// push 已按位补 IPI，置位后的 push 经 Acquire 读必见本位置位——
/// AcqRel 与 wake_sleepers 的 Acquire 配对闭合「漏唤醒」窗口。
pub(super) fn mark_sleeping(hart: usize) {
    SLEEPING.fetch_or(1usize << hart, Ordering::AcqRel);
}

/// 清除 hart 的休眠标记（WFI 唤醒后 / 复查发现任务时调用）。
pub(super) fn mark_awake(hart: usize) {
    SLEEPING.fetch_and(!(1usize << hart), Ordering::AcqRel);
}

/// 唤醒所有 WFI 休眠中的 hart（push 后调用：新任务出现 → 睡核可 steal）。
///
/// SBI IPI 把目标核 SSIP 置位（sie.SSIP 已使能），目标核在 WFI 中挂起即被唤醒
/// ——全局 SIE=0，只唤醒不取中断。错误（如核已醒）尽力而为。
pub(super) fn wake_sleepers() {
    let sleeping = SLEEPING.load(Ordering::Acquire);
    if sleeping == 0 {
        return;
    }
    // a0 = hart_mask（≤ 8 核，mask_base = 0）
    let _ = ecall::IpiCall::new(fid::Ipi::SendIpi)
        .args(SArgs {
            a0: sleeping,
            ..Default::default()
        })
        .call();
}
