// 票与票根（holder）——挂起的身份（票）与「凭票认人」的票根表。

use alloc::sync::{Arc, Weak};
use core::sync::atomic::{AtomicUsize, Ordering};

use hashbrown::HashMap;

use crate::lock::{Level, OnceLock, SpinLock};
use crate::runtime::chrono::timer;
use crate::work::unit::task::Task;

use super::site::WakeKey;

// ── 票与票根 ──

/// 票：一次挂起的唯一标识。单调、不复用。
///
/// 到点登记（`timer::tock`）只携带它。凭它可以还原出「谁**在哪个键上**等」——
/// `HOLDERS` 存的就是这一对（键 + 持票人）。**键存在票根里而不是任务 payload 里**，
/// 是为了让到点路径（`redeem`）不必读任务的 `state`：它是**观察者**（持票人那份 Arc
/// 是临时的，任务随时可能被别核 `wipe`/`suspend` 放行）⇒ 读 payload 就是读一个被
/// 独占写的字段。问容器就没有这个问题。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ticket(pub(super) u64);

impl Ticket {
    /// 发票。Relaxed 足够：票号只用于相等判定，不承载顺序。
    pub(super) fn alloc() -> Ticket {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        Ticket(NEXT.fetch_add(1, Ordering::Relaxed) as u64)
    }

    /// 到点登记用的裸值。
    pub(super) fn raw(self) -> u64 {
        self.0
    }
}

/// 票根表：票 →（唤醒键, 持票人）。
type HolderTable = SpinLock<HashMap<Ticket, (WakeKey, Weak<Task>)>>;

/// 票根：票 → 持票人。**只存 `Weak`**。
///
/// 挂起任务的强持有者只能是它所在的站点队列（见模块头的「唯一强持有」）。这里若
/// 存 `Arc`，任务就有了第二个强持有者：一撞 `Task::exclusive` 的唯一性前提，二让
/// 陈旧的到点登记把已回收的任务钉住（关机审计会把它报成帧泄漏）。
pub(in super::super) fn holders() -> &'static HolderTable {
    static HOLDERS: OnceLock<HolderTable> = OnceLock::new();
    HOLDERS.get_or_init(|| SpinLock::new_level(Level::L3, HashMap::new()))
}

/// 存根。**前置：到点登记尚未发生**——「堆可见 ⇒ 票根必在」，否则到期路径会命中
/// 一个空的票根。
pub(super) fn hold(ticket: Ticket, key: WakeKey, task: &Arc<Task>) {
    holders().lock().insert(ticket, (key, Arc::downgrade(task)));
}

/// 作废票根并取回持票人：到期认领与提前作废走同一条路，**幂等**（票号不复用，
/// 第二次必得 `None`）。顺带消音它的到点——`timer::mute` 对已取走的句柄是 no-op，
/// 故到期路径重复调用也无害（代价是一次空扫，n = 未到点 tock 数）。
pub(in super::super) fn void(ticket: Ticket) -> Option<(WakeKey, Arc<Task>)> {
    timer::mute(ticket.raw());
    holders()
        .lock()
        .remove(&ticket)
        .and_then(|(key, w)| w.upgrade().map(|t| (key, t)))
}
