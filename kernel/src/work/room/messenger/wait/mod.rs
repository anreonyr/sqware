// 等待机（wait）——「挂起 → 唤醒」这条链对外的几个入口：park / wait / join / wake /
// wipe / redeem，共用一条挂起实现 `block`。
//
// 站点表（唯一容器）与票根分居 `site` / `holder`；这两个子模块里跨到 `messenger`
// 一级的条目取 `pub(in super::super)`——刚好到 `messenger`，不放宽到 `pub(crate)`。

pub(super) mod holder;
pub(super) mod site;

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::time::Duration;

use crate::runtime::chrono::{clock, timer};
use crate::runtime::diagnose::trace::{self, EventKind, RoomEvent};
use crate::work::room::conductor;
use crate::work::room::scheduler::core::current;
use crate::work::room::scheduler::trap::run;
use crate::work::unit::gate::GateError;
use crate::work::unit::task::{Task, TaskState};

use self::holder::{Ticket, hold, void};
use self::site::{Site, Waiter, WakeKey, prune, sites, take_beacon};
use super::handoff::Handoff;

// ── 操作：挂起（用 scheduler::core::Scheduler::disown_and_install_next） ──

/// 挂起的唯一实现：三处入口（`park` / `wait` / `join`）只差一个键。
///
/// 时序（两个竞态闭合点）：
///   ① 信标先探——信号已至 → 不挂起（不碰站点表：缺键即无信标）
///   ② 离核——借 scheduler 跨边界原语把 running 卸下（槽位 settled）
///   ③ 登记——发票 → 存票根 → `tock`（**先票根后 tock**：堆可见 ⇒ 票根必在）
///   ④ 入队——写等待点 + 挂进站点队列；**锁内再查一次信标**
///   ⑤ 窗口内信号已至 → 撤销登记，按「已唤醒」处理（Starved 入队）
///
/// 锁纪律：站点表与票根都是 L3，**绝不互相嵌套**——「作用域内取、作用域外用」。
fn block(key: WakeKey, dur: Duration) -> Handoff<()> {
    // ① 信标先探
    if take_beacon(key) {
        return Handoff::Resume(());
    }
    // ② 离核
    let (mut task, next_pa) = current().disown_and_install_next();
    // ③ 登记
    let ticket = Ticket::alloc();
    let at = (dur != Duration::MAX).then(|| clock::now().add(dur).as_ticks());
    if let Some(at) = at {
        hold(ticket, &task);
        timer::tock(ticket.raw(), at);
    }
    trace::note(EventKind::Room(RoomEvent::Wait {
        tid: task.ident.id,
        // 诊断用折叠值：键成枚举后不再有「人可读的位打包」形态。
        key: key.fold() as usize,
    }));
    // ④ 写等待点 + 入队（锁内查信标）
    Task::exclusive(&mut task).transform(TaskState::Blocked { key, ticket });
    let queued = {
        let mut sites = sites(key).lock();
        let site = sites.entry(key).or_insert_with(Site::new);
        let queued = if site.pend {
            site.pend = false;
            false
        } else {
            site.waiters.push_back(Waiter {
                task: task.clone(),
                ticket,
            });
            true
        };
        // 撤销阻塞那一支没有留下等待者：空壳站点随手删掉。
        prune(&mut sites, key);
        queued
    };
    // ⑤ 窗口内信标已至：撤销登记，按已唤醒处理
    if !queued {
        void(ticket);
        rise(core::iter::once(task));
    }
    // 本核无后继即就地取活：`run()` 只会循环到有帧或停机，故落点恒为 `Switch`。
    Handoff::Switch(next_pa.unwrap_or_else(run))
}

/// 放回就绪——「唤醒」的全部效果就是这一件事。
///
/// `wake` / `wipe` / `redeem` 与撤销阻塞四条路径的收尾完全同形（置 Starved →
/// 记事件 → 推回本核 starved 队列），故只写一遍。`kick` 提到批量之后：push 先于
/// 踢，唤醒方进入 steal 必可见（单 tick 的 IPI 量从 O(N) → O(1)）。返回唤醒数。
fn rise<I: IntoIterator<Item = Arc<Task>>>(tasks: I) -> usize {
    let mut woke = 0;
    for task in tasks {
        let mut t = task;
        Task::exclusive(&mut t).transform(TaskState::Starved);
        trace::note(EventKind::Room(RoomEvent::Wake { tid: t.ident.id }));
        current().push(t);
        woke += 1;
    }
    if woke > 0 {
        conductor::kick();
    }
    woke
}

/// 纯睡（`RoomCall::Park`）：键是 `Alarm { 我 }`——无人投信，只有期限会响。
///
/// Running → Blocked；返回下一帧 PA（若 scheduler 装了下一 starved）。
pub fn park(duration: Duration) -> usize {
    let me = current()
        .running_task()
        .expect("park: no running task")
        .ident
        .id;
    let wake_at = clock::now().add(duration).as_ticks();
    trace::note(EventKind::Room(RoomEvent::Park {
        tid: me,
        wake_at: wake_at as usize,
    }));
    match block(WakeKey::Alarm { task: me }, duration) {
        Handoff::Switch(pa) => pa,
        // `Alarm` 无投信方 ⇒ 信标先探不可能命中。
        Handoff::Resume(()) => unreachable!("Alarm 无投信方"),
    }
}

/// 事件等待（`RoomCall::Wait`）：直通 [`block`]。有投信方的键，信标先探可能命中
/// 而当场续跑（[`Handoff::Resume`]）。
pub fn wait(key: WakeKey, dur: Duration) -> Handoff<()> {
    block(key, dur)
}

// ── 操作：等目标回收（Join） ──

/// 目标是否已死透。注册表只存 `Weak` 且从不清理：升级失败 ⇒ 已分配过就是
/// 「已回收」；从未分配 ⇒ 非法 id（调用方另判 `Denied`）。
///
/// `Reaped` 由 `reap` 独占置位（钩子之后），故本判据为真 ⇔ **收尾已完成**。
fn target_dead(tid: usize) -> bool {
    match crate::work::room::scheduler::core::lookup_task_by_id(tid) {
        Some(t) => t.state() == TaskState::Reaped,
        None => crate::work::unit::task::allocated(tid),
    }
}

/// 等目标结束（`Join` 的承载）。
///
/// 契约（与 `wait`/`pull` 同源）：**未挂起**时结论精确；**挂起过**则恢复后读到的
/// a0 是挂起前预置值（内核没有第二次执行机会），调用方须复探 `Join{task, 0}`。
/// 唤醒由**内核驱动**——目标收尾（含 fault isolation 杀）时 [`wipe`]
/// 放行全部等待者，故用户态跑不到的死亡也能被观察到。
///
/// 「结束」= 目标已死**且退出钩子（通道级联 + 能力级联）已跑完**——即返回真时，
/// 它名下的门闩与通道都已消失。栈/trap 帧/团队空间的回收是内核私事、对调用方
/// 不可观测，故**不入契约**（那也是延迟回收存在的理由）。
pub fn join(tid: usize, dur: Duration) -> Result<Handoff<bool>, GateError> {
    if target_dead(tid) {
        return if crate::work::unit::task::allocated(tid) {
            Ok(Handoff::Resume(true))
        } else {
            Err(GateError::Denied)
        };
    }
    if dur == Duration::ZERO {
        return Ok(Handoff::Resume(false));
    }
    Ok(match block(WakeKey::Task { id: tid }, dur) {
        Handoff::Switch(pa) => Handoff::Switch(pa),
        // 信标已置：目标在「判死 → 入队」的窗口内被回收 ⇒ 当场结论（已回收）。
        Handoff::Resume(()) => Handoff::Resume(true),
    })
}

/// 键退役：放行该键上的**全部**等待者，并留下信标（墓碑）。
///
/// 「目标已回收」与「资源已封印/销毁」是同一件事的两副面孔：这个键再也不会有人
/// 投信，此后到达的等待者必须**当场**得到结论，而不是永远等下去——信标承担这一点
/// （`wake` 只在无人在等时置位；这里无条件置位）。一个信标够用：调用方在入队前都
/// 先探过条件（`join` 探 `target_dead`、`hole::wait` 探就绪位），窗口最多一人。
///
/// 锁纪律同 [`wake`]：只在站点表（L3）内摘除，锁外 transform + 入队。返回唤醒数。
pub(crate) fn wipe(key: WakeKey) -> usize {
    let waiters = {
        let mut sites = sites(key).lock();
        let site = sites.entry(key).or_insert_with(Site::new);
        site.pend = true;
        core::mem::take(&mut site.waiters)
    };
    for w in &waiters {
        void(w.ticket);
    }
    rise(waiters.into_iter().map(|w| w.task))
}
// ── 操作：唤醒 ──
// ── 操作：唤醒 ──

/// 叫醒一个：摘队首 → 放回就绪。无人在等 → 置信标（防漏唤醒）。返回是否唤到人。
///
/// 消费方 = utask/envcall 与 mail 的投信方；跨核经 steal 再平衡（同 [`redeem`]）。
///
/// **信标可能陈旧**：「信号」与「数据」是两份状态——等待者后来直接取走数据
/// （裸 pull 成功，不经 `wait`）时信标不被消费，下一次 `wait` 就立刻返回「已唤醒」
/// 而实际无数据。故 `wait` 的返回**只是提示**，调用方必须自己复核条件
/// （`hole::wait` 已复核就绪位；有界等待方还须按 deadline 循环，见
/// `docs/dispatch.md` §11.4）。
pub fn wake(key: WakeKey) -> bool {
    let popped = {
        let mut sites = sites(key).lock();
        let site = sites.entry(key).or_insert_with(Site::new);
        let popped = match site.waiters.pop_front() {
            Some(w) => Some(w),
            None => {
                site.pend = true;
                None
            }
        };
        prune(&mut sites, key);
        popped
    };
    let Some(w) = popped else { return false };
    void(w.ticket);
    rise(core::iter::once(w.task));
    true
}

/// 到期兑现：`chrono` 交回的不透明句柄，在这里还原成「谁」。
///
/// 一段走完，不再认识 park / wait / join 的区别：
///   票根 → 升出持票人 → 读它自己那张票上的键 → 从该键的队列里按票号摘出。
/// 陈旧的登记在每一步都自然落空（票根已被 `void`、任务已不阻塞、票号对不上），
/// 故不需要任何「取消」记账。
///
/// 按 tock 堆取到期者（与入队顺序无关）；堆锁先放后取，绝不持堆锁取调度锁
/// （防 ABBA）。返回：本次是否撤出过任务（空闲核的哑睡壳判定用）。
/// 由 trap 路径（S-timer 处理）与空闲核归队时在本 hart 触发。
pub fn redeem() -> bool {
    let due = timer::drain(clock::now());
    // 批量收集再统一 `rise`：一次 kick 收尾（逐条踢会让 IPI 量回到 O(N)）。
    let mut tasks: Vec<Arc<Task>> = Vec::new();
    for handle in due {
        // 票号即到点登记的身份：作废票根并取回持票人（已回收 → 落空）。
        let Some(task) = void(Ticket(handle)) else {
            continue;
        };
        let TaskState::Blocked { key, ticket } = task.state() else {
            continue;
        };
        // 从该键的队列里摘出**这一票**的等待者（票号对不上 = 陈旧，落空）。
        let popped = {
            let mut sites = sites(key).lock();
            let w = sites.get_mut(&key).and_then(|site| {
                site.waiters
                    .iter()
                    .position(|w| w.ticket == ticket)
                    .map(|i| site.waiters.remove(i).expect("idx from position"))
            });
            prune(&mut sites, key);
            w
        };
        let Some(w) = popped else { continue };
        tasks.push(w.task);
    }
    rise(tasks) > 0
}
