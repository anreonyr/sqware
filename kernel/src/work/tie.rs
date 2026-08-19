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
/// WFI 等待唤醒的 hart 位图（字 w 的 bit b = hart `w·64 + b` 正阻塞在
/// scheduler::wait；push 后据此发 IPI——休眠核醒来后可 steal 新任务）。
/// 多字：字宽 = SBI 单次 `sbi_send_ipi` 的寻址窗口（XLEN = 64 位），按
/// MAX_HART_SLOTS 分字（当前 4096/64 = 64 字 = 512 B 静态）。位位置 = hartid，
/// 故仍要求 DTB hartid 连续（QEMU 满足；稀疏 hartid 需映射层，超出本次范围）。
static WAITING: [AtomicUsize; WAITING_WORDS] = [const { AtomicUsize::new(0) }; WAITING_WORDS];

/// WAITING 位图字数：每字 64 位（= 协议单次 IPI 掩码窗口）。
const WAITING_WORDS: usize = crate::machine::MAX_HART_SLOTS / usize::BITS as usize;

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
        // 全部核已到齐（回收完毕）、帧已归还——先抽干每个块的 pump（过境块全部归位，
        // 帧基线扣除公式才成立），再断言零泄漏，最后发复位（debug 构建生效）。
        crate::memory::allocator::block::flush_all();
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
    debug_assert!(
        hart < crate::machine::MAX_HART_SLOTS,
        "sleep hart {hart} beyond MAX_HART_SLOTS"
    );
    WAITING[hart / (usize::BITS as usize)].fetch_or(1usize << (hart % (usize::BITS as usize)), Ordering::AcqRel);
}

/// 清除 hart 的等待标记（WFI 唤醒后 / 复查发现任务时调用）。
pub(super) fn wake(hart: usize) {
    debug_assert!(
        hart < crate::machine::MAX_HART_SLOTS,
        "wake hart {hart} beyond MAX_HART_SLOTS"
    );
    WAITING[hart / (usize::BITS as usize)].fetch_and(!(1usize << (hart % (usize::BITS as usize))), Ordering::AcqRel);
}

/// 唤醒所有 WFI 等待中的 hart（push 后调用：新任务出现 → 睡核可 steal）。
///
/// SBI IPI 把目标核 SSIP 置位（sie.SSIP 已使能），目标核在 WFI 中挂起即被唤醒
/// ——全局 SIE=0，只唤醒不取中断。错误（如核已醒）尽力而为。
///
/// 按 64 核一组循环：SBI 协议每次 `sbi_send_ipi` 的掩码至多 XLEN(=64) 位，超过
/// 必须以 mask_base 递增多次调用（riscv-sbi-doc binary-encoding 的 Hart list
/// parameter 节明确如此规定，未设总核数上限）。当前字 0 之外恒为空，
/// 循环退化回单次调用。
pub(super) fn wake_all() {
    for (w, word) in WAITING.iter().enumerate() {
        let waiting = word.load(Ordering::Acquire);
        if waiting == 0 {
            continue;
        }
        // a0 = hart_mask（本组 64 位，bit b = hart w·64+b），a1 = mask_base = w·64
        let _ = sbi::IpiCall::new(fid::Ipi::SendIpi)
            .args(SArgs {
                a0: waiting,
                a1: w * (usize::BITS as usize),
                ..Default::default()
            })
            .call();
    }
}
