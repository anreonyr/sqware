// 系统级生命周期：跨核共享的原子状态与编排（不属任何单核调度器）。
//
// 两件事：
//   全退出停机 — PUSHED/REAPED 任务计数（入队 +1 / 回收 +1）；相等且 PUSHED>0
//              （防 boot 早期误判）即全部退出 → halt 发 SBI srst 复位；
//              HALTING 做一次性互斥，防多核同时发复位（后到者 wfi 等复位）。
//   休眠唤醒   — WAITING 位图（bit h = hart h 正 WFI 等待）；入队后 wake_all
//              按掩码一次 SBI IPI 唤醒全部睡核（可来 steal）；置位/清位经
//              sleep/wake（AcqRel），配合调用方的置位后复查闭合
//              「漏唤醒」窗口（见 scheduler::wait）。
//
// 命名与 scheduler 对齐：动词 = 目标状态名词（spawn/exit/done/halt/sleep/wake/
// wake_all/wfi）；静态名词按 scheduler 调用点命名（PUSHED ← scheduler::push、
// REAPED ← scheduler::reap、WAITING ← scheduler::wait；HALTING 无对应词保留）。

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use sbi::scall::SArgs;
use sbi::{self, fid};
use crate::machine;
use crate::memory::allocator::frame;
use crate::putln;

/// 已入队（创建）任务计数（全退出检测：REAPED == PUSHED → 停机）。
static PUSHED: AtomicUsize = AtomicUsize::new(0);
static REAPED: AtomicUsize = AtomicUsize::new(0);
/// 停机互斥：第一个触发 srst 的核胜出，其余 wfi（避免双 srst）。
static HALTING: AtomicBool = AtomicBool::new(false);
/// 已到达 halt 的核数 — 关机屏障：胜出核须等**全部**核到达后再断言帧基线。
/// 其它核可能在 done() 置位瞬间仍在 clear()/task_reclaim 归还任务帧，不等齐
/// 会把在途回收误报为泄漏（多核 shutdown 竞态）。
static HALT_ARRIVED: AtomicUsize = AtomicUsize::new(0);
/// WFI 等待唤醒的 hart 位图（bit h = hart h 正阻塞在 scheduler::wait；push 后
/// 据此发 IPI——休眠核醒来后可 steal 新任务）。
static WAITING: AtomicUsize = AtomicUsize::new(0);

/// 任务入队计数 +1（scheduler::push 收尾时调用；PUSHED）。
///
/// Relaxed 够用：计数只用于相等比较，且自增发生在持本核调度锁时。
pub(super) fn push() {
    PUSHED.fetch_add(1, Ordering::Relaxed);
}

/// 任务回收计数 +1（scheduler::reap 清理后调用；REAPED）。
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
/// 关机屏障：先登记本核到达（HALT_ARRIVED），胜出核唤醒 WFI 睡核（它们随后
/// 也会到达 halt）并**自旋等全部核到齐**，然后才经 `frame::check_baseline` 断言
/// 任务帧已全部归还（在途帧回落内核持久帧 + 堆支撑页基线）——地址空间/栈
/// 所有权 Drop 泄漏在此暴露。不等齐的核可能仍在 clear() 归还帧，会把在途回收
/// 误报为泄漏。仅胜出核检查一次即可（原子互斥保证单核执行）。
pub(super) fn halt() -> ! {
    HALT_ARRIVED.fetch_add(1, Ordering::AcqRel);
    if !HALTING.swap(true, Ordering::AcqRel) {
        // 唤醒 WFI 睡核：它们醒来后同样会走 done → halt → 登记到达。
        wake_all();
        while HALT_ARRIVED.load(Ordering::Acquire) < machine::hart_count() {
            core::hint::spin_loop();
        }
        putln!("task: all tasks exited, system halted");
        // 全部核已到齐（回收完毕）、帧已归还——断言零泄漏后再发复位（debug 构建生效）。
        #[cfg(debug_assertions)]
        frame::check_baseline();
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

/// 标记 hart 进入 WFI 等待（scheduler::wait 的「阻塞点」）。调用方须在置位后
/// **复查队列**再睡：置位前的 push 已按位补 IPI，置位后的 push 经 Acquire 读
/// 必见本位置位——AcqRel 与 wake_all 的 Acquire 配对闭合「漏唤醒」窗口。
pub(super) fn sleep(hart: usize) {
    WAITING.fetch_or(1usize << hart, Ordering::AcqRel);
}

/// 清除 hart 的等待标记（WFI 唤醒后 / 复查发现任务时调用）。
pub(super) fn wake(hart: usize) {
    WAITING.fetch_and(!(1usize << hart), Ordering::AcqRel);
}

/// 唤醒所有 WFI 等待中的 hart（push 后调用：新任务出现 → 睡核可 steal）。
///
/// SBI IPI 把目标核 SSIP 置位（sie.SSIP 已使能），目标核在 WFI 中挂起即被唤醒
/// ——全局 SIE=0，只唤醒不取中断。错误（如核已醒）尽力而为。
pub(super) fn wake_all() {
    let waiting = WAITING.load(Ordering::Acquire);
    if waiting == 0 {
        return;
    }
    // a0 = hart_mask（≤ 64 核，mask_base = 0 —— WAITING 位图与 SBI IPI 掩码同为
    // 64 位，MAX_HARTS = usize::BITS 即此协议边界）
    let _ = sbi::IpiCall::new(fid::Ipi::SendIpi)
        .args(SArgs {
            a0: waiting,
            ..Default::default()
        })
        .call();
}
