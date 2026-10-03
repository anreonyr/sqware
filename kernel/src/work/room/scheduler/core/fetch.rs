use alloc::sync::Arc;

use riscv::register::{sie, sip};

use crate::hart;
use crate::runtime::chrono::timer;
use crate::work::room::conductor;
use crate::work::room::messenger;
use crate::work::unit::task::Task;

use super::table::current;

const WFI_FAR: u64 = 1 << 60;
const BEACON_TICK: u64 = 250_000;

pub(in super::super) fn fetch() -> usize {
    let s = current();
    loop {
        // **开闸门**：外部门闸（`SEIE`）是**按 hart 记账**的——取到外部中断而
        // `raise_irq()` 失败时，`trap` 那一支会关掉**本 hart** 的闸门，而重开它的路只有两条：
        // 本 hart 上有人 `hush`（`envcall/mail.rs` 那一支），或**本 hart 进空闲循环**
        //（`wait()` 那一支，每轮无条件置）。
        //
        // 于是"**一直有活干的 hart**"那一条路上**没有**重开的点：量到的那一族
        //（整机跑完不出场：喂进来的字节再也不被处理、中断计数冻住）就是这么来的——
        // 关闸门的那颗 hart 被调度一直占着（编排域那圈 1 ms 复问），永不空闲 ⇒ 闸门永不再开。
        // 这一句与 `wait()` 那一支对称：**跑任务与空闲两种长驻态各有振铃点**（`bell.rs` 头注
        // 那句说的是意图，这里是它的另一半）。多开一次无害：真响着的时候那一枚铃本来就 `Busy`。
        // SAFETY: 只置 sie.SEIE 一位，不改任何内存与栈
        unsafe {
            sie::set_sext();
        }
        if let Some(task) = s.pull() {
            if let Some(pa) = s.seat(task) { return pa; }
        }
        if conductor::done() {
            conductor::halt();
        }
        if let Some(task) = wait() {
            if let Some(pa) = s.seat(task) { return pa; }
        }
    }
}

fn wait() -> Option<Arc<Task>> {
    let me = hart::hart_id();
    conductor::sleep(me);
    let found = current().pull();
    if let Some(task) = found {
        conductor::wake(me);
        return Some(task);
    }
    if conductor::done() {
        conductor::halt();
    }
    loop {
        if conductor::done() {
            // SAFETY: 写本 hart 自己的 sip CSR，仅清 SSIP 位
            unsafe { sip::clear_ssoft() };
            conductor::wake(me);
            conductor::halt();
        }
        crate::work::room::scheduler::core::beacon::idle(me);

        let fallback = if crate::work::room::scheduler::core::beacon::shutting_down() {
            BEACON_TICK
        } else {
            WFI_FAR
        };
        timer::beat_until(fallback);
        // SAFETY: 只置 sie.SEIE 一位，不改任何内存与栈
        unsafe {
            sie::set_sext();
        }
        if sip::read().sext() {
            let _ = crate::platform::devices::raise_irq_idle();
        }
        #[cfg(debug_assertions)]
        crate::runtime::diagnose::ipi::idle_hook();
        #[cfg(debug_assertions)]
        crate::runtime::diagnose::ipi::wfi_entry(me);
        unsafe {
            core::arch::asm!("wfi");
        }
        #[cfg(debug_assertions)]
        crate::runtime::diagnose::ipi::wfi_exit(me, sip::read().ssoft());
        crate::work::room::messenger::sweep_doomed();
        if messenger::redeem() {
            break;
        }
        if let Some(task) = current().pull() {
            conductor::wake(me);
            return Some(task);
        }
        // SAFETY: 写本 hart 自己的 sip CSR，仅清 SSIP 位
        unsafe { sip::clear_ssoft() };
    }
    // SAFETY: 写本 hart 自己的 sip CSR，仅清 SSIP 位
    unsafe { sip::clear_ssoft() };
    conductor::wake(me);
    None
}
