// 取活（core::fetch）— 本核没活时怎么拿到活：跨核偷取 + WFI 休眠。
//
// `fetch` 是「Idle → Running」的对外入口，顺序**不可重排**：全退出检查须在 steal
// **之前**（停机后不得再取活）；`wait` 内部自带全退出复审 + 睡眠位协议，且它睡下
// 之前与醒来之后都要再取一次活（防「检查完 → 置位 → 睡」窗口内的入队漏唤醒）。
//
// `steal` 与 `wait` 都只在本文件内用（对外只有 `fetch`）。

use alloc::sync::Arc;
use core::sync::atomic::Ordering;

use riscv::register::{sie, sip};

use crate::machine;
use crate::runtime::chrono::{clock, timer};
use crate::runtime::diagnose::trace::{self, EventKind, RoomEvent};
use crate::work::room::conductor;
use crate::work::room::messenger;
use crate::work::unit::task::Task;

use super::table::{current, schedulers};

/// WFI 休眠的推远增量：无待唤醒 tock 时 arm 到「永远」。
const WFI_FAR: u64 = 1 << 60;

/// 取活：本核队首 → 全退出 → 跨核偷 → 睡下等。返回下一帧 PA（永不返回 None：
/// 全退出时走 `conductor::halt`）。
pub(in super::super) fn fetch() -> usize {
    let s = current();
    loop {
        if let Some(task) = s.pull() {
            return s.seat(task);
        }
        if conductor::done() {
            conductor::halt();
        }
        if let Some(task) = steal() {
            return s.seat(task);
        }
        if let Some(task) = wait() {
            return s.seat(task);
        }
    }
}

/// 非阻塞偷取：先读 starved_len（锁外原子读，S 态共享不失效缓存行）——空队列
/// 不做 RMW，避免对受害者锁行乒乓；有活才 try_lock（失败即跳过——victim 忙时
/// 不等待，无锁序规则）。锁内 pull 复查队列防竞态。
///
/// 起点随机化：每核持 `steal_cursor` 本地 fetch_add(1) % hart_count 派生起点，
/// 多核同时醒来时各 hart 起点天然分散——避免全从 hart 0 起步造成的 cache
/// 热点（多 hart 同时对同目标的 L1 锁 RMW → cache line 乒乓 = 雷鸣群）。
fn steal() -> Option<Arc<Task>> {
    let me = machine::hart_id();
    let n = machine::hart_count();
    if n <= 1 {
        return None;
    }
    // 每核独立游标派生起点：fetch_add 是 Relaxed，无内存序代价。
    let start = current().steal_cursor.fetch_add(1, Ordering::Relaxed) % n;
    for off in 0..n {
        let v = (start + off) % n;
        if v == me {
            continue;
        }
        if schedulers()[v].backlog() == 0 {
            continue;
        }
        let Some(task) = schedulers()[v].try_pull() else {
            continue;
        };
        trace::note(EventKind::Room(RoomEvent::Steal {
            tid: task.ident.id,
            src_hart: v,
        }));
        return Some(task);
    }
    None
}

/// 本 hart 进入 WFI 休眠（Idle 自环的「阻塞点」）。`Some` = 睡醒后有活；
/// `None` = 到期兑现（`redeem`）放行了别人的任务，本核没拿到——交回 `fetch` 复审。
///
/// 协议：置睡眠位 → 复查（防 push 漏唤醒）→ 全退出检查 → 睡到最近 tock → WFI。
/// 唤醒后：有任务 → 正常出口；到期假醒但无活 → 哑睡壳回睡（保持睡眠位、不打点不清位）。
fn wait() -> Option<Arc<Task>> {
    let me = machine::hart_id();
    conductor::sleep(me);
    // 置位后复查：防「检查完 → 置位 → 睡」窗口内的 push 漏唤醒
    let found = current().pull().or_else(steal);
    if let Some(task) = found {
        conductor::wake(me);
        return Some(task);
    }
    if conductor::done() {
        conductor::halt();
    }
    loop {
        // 每次决定重新睡下前，先复审全退出：halt 的 yell 会把本核从 WFI 拉起。
        // 若这里不归队 halt，而 redeem 又无可唤醒任务、steal 也无活，
        // 就会清 SSIP 后回睡，停机屏障将永远等不到本核的 HALT_ARRIVED。
        if conductor::done() {
            // SAFETY: 写本 hart 自己的 sip CSR，仅清 SSIP 位，无并发别名。
            unsafe { sip::clear_ssoft() };
            conductor::wake(me);
            conductor::halt();
        }

        let delta = match timer::due() {
            Some(t) => t.as_ticks().saturating_sub(clock::now().as_ticks()),
            None => WFI_FAR,
        };
        timer::beat(delta);
        // 外部中断的闸门也在这里重开：「下一拍无条件重开」那条只管**以陷阱形式取到**
        // 的 timer tick，而空闲核的拍子是在这里处理的——SIE=0 的 WFI 只被"挂起"唤醒，
        // 不进陷阱。少了这一句，关过闸门的空闲核就再也不会重开（`docs/driver.md`
        // §3.2.2 的"自愈"在空闲核上不成立）。
        //
        // **挂着的就别开**：SEIP 还置着说明那条中断没人领（多半是槽满被闸门挡下的），
        // 此刻重开只会让 WFI 立刻返回、把空闲核变成一个探测死循环——而它挂在 WFI 上
        // 才是对的（认领是消费者的事）。等重开条件自然成立（消费者把 pending 领走、
        // SEIP 落下）下一轮就开。
        // SAFETY: 只置 sie.SEIE 一位，不改任何内存与栈。
        if !sip::read().sext() {
            unsafe {
                sie::set_sext();
            }
        }
        // WFI：SSIP（IPI）/ STIP（定时器到期）挂起即唤醒——只唤醒不取中断（SIE=0）。
        // 注意：不再有清退应答点——RFENCE 由固件强制打断空闲核（含 WFI 态），
        // 目标核进 trap 执行 sfence，无需空闲核主动 sweep。
        unsafe {
            core::arch::asm!("wfi");
        }
        // timer 到期分派由 messenger 处理（票根 → 键 → 站点，一路）
        if messenger::redeem() {
            break;
        }
        // 假醒：也可能被 yell 的 IPI 唤来 steal（有活入队）——先复查取活，
        // 有任务即正常出口（睡眠位就在本分支清掉，见下）；真无活才保持睡眠位回睡。
        if let Some(task) = current().pull().or_else(steal) {
            conductor::wake(me);
            return Some(task);
        }
        // 哑睡壳（假醒无活）：保持睡眠位、不打点不清位，清残留 SSIP 后回睡。
        // SAFETY: 写本 hart 自己的 sip CSR，仅清 SSIP 位，无并发别名。
        unsafe { sip::clear_ssoft() };
    }
    // 正常出口：清 SSIP（防残留位导致下次 WFI 立即重醒）与睡眠位
    // SAFETY: 写本 hart 自己的 sip CSR，仅清 SSIP 位，无并发别名。
    unsafe { sip::clear_ssoft() };
    conductor::wake(me);
    None
}
