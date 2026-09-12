// 等待机（wait）——「挂起 → 唤醒」这条链对外的几个入口：park / wait / join / wake /
// wipe / redeem，共用一条挂起实现 `block`。
//
// 站点表（唯一容器）与票根分居 `site` / `holder`；这两个子模块里跨到 `messenger`
// 一级的条目取 `pub(in super::super)`——刚好到 `messenger`，不放宽到 `pub(crate)`。

pub(super) mod holder;
pub(super) mod site;

use alloc::collections::VecDeque;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::time::Duration;

use crate::runtime::chrono::{clock, timer};
use crate::runtime::diagnose::trace::{self, EventKind, RoomEvent};
use crate::work::room::conductor;
use crate::work::room::scheduler::core::current;
use crate::work::room::scheduler::trap::run;
use crate::work::unit::life::{Life, TaskLife};
use crate::work::unit::task::{Task, TaskState};

use self::holder::{Ticket, hold, void};
use self::site::{SITE_SHARDS, Site, Waiter, WakeKey, prune, shard_at, sites, take_beacon};
use super::handoff::Handoff;

// ── 操作：挂起（用 scheduler::core::Scheduler::swap） ──

/// 挂起的唯一实现：三处入口（`park` / `wait` / `join`）只差一个键。
///
/// 时序（两个竞态闭合点）：
///   ① 信标先探——信号已至 → 不挂起（不碰站点表：缺键即无信标）
///   ② 离核——借 scheduler 跨边界原语把 running 卸下（槽位 settled）
///   ③ 登记——发票 → 存票根 → `tock`（**先票根后 tock**：堆可见 ⇒ 票根必在）
///   ④ 入队——写等待点 + 挂进站点队列；**锁内先判键死活、再查一次信标**
///   ⑤ 窗口内信标已至或键已死 → 撤销登记，按「已唤醒」处理（Starved 入队）
///
/// `life` = 本键的存活单元（弱引用，调用方随键一起交进来——room 不查任何注册表）。
/// ④ 的锁内判死就是 A2 说的「关上在飞窗口」：一个正飞在 ①④ 之间的等待者，此前
/// 只能靠 `wipe` 留下的墓碑接住；现在键自己会答（`weak.upgrade` 失败），于是墓碑
/// 可以不留。键已死这一支**走既有回滚**（⑤ 的 `void(ticket)` + `rise`），不新增
/// 任何清理机制——`Blocked` 只在 push 那一支被写，状态仍与容器一致。
///
/// 锁纪律：站点表与票根都是 L3，**绝不互相嵌套**——「作用域内取、作用域外用」。
fn block(key: WakeKey, life: Weak<Life>, dur: Duration) -> Handoff<()> {
    // ① 信标先探
    if take_beacon(key) {
        return Handoff::Resume(());
    }
    // ② 离核
    let (mut task, next_pa) = current().swap();
    // ③ 登记
    let ticket = Ticket::alloc();
    let at = (dur != Duration::MAX).then(|| clock::now().add(dur).as_ticks());
    if let Some(at) = at {
        hold(ticket, key, &task);
        timer::tock(ticket.raw(), at);
    }
    trace::note(EventKind::Room(RoomEvent::Wait {
        tid: task.ident.id,
        // 诊断用折叠值：键成枚举后不再有「人可读的位打包」形态。
        key: key.fold() as usize,
    }));
    // ④ 写等待点 + 入队（锁内判死活 + 查信标）
    Task::exclusive(&mut task).transform(TaskState::Blocked { key, ticket });
    let queued = {
        let mut sites = sites(key).lock();
        let site = sites.entry(key).or_insert_with(|| Site::new(&life));
        // 站点带着**本键**的存活单元：同一个键只有一份 Life，故这枚弱引用与入口
        // 无关（wait / join 指同一个分配），赋值不是「换主」而是「同一事实的重写」。
        //
        // **按值移进站点**（不是 `clone()`）：这一行之后，本帧与调用链上再没有这枚
        // 弱引用的副本。跨挂起的引用只要还在某个局部量里，那条链一旦被弃（被别核判死 /
        // 收尾时就地冻住）它的 `Drop` 就永不执行 —— `block` 头注里那条"跨挂起不得持
        // 强引用"的纪律，对**弱引用**同样成立（弱引用不钉载荷、钉的是外壳）。
        site.life = life;
        let queued = if Life::dead(&site.life) {
            // 键已死（资源没了）：不入队、也不留站点——死键的队列必然空（能入队 ⇒
            // 入队那一刻键还活着），故下面的 `prune` 会当场把这个空壳删掉。
            // 与「信标已至」同一支收尾（⑤ 的 `void` + `rise`）：两支的对外结论都是
            //「没挂起、当场放回」，调用方本来就须复核条件（信标只是提示）。
            false
        } else if site.pend {
            // 窗口内信标已至：消费它，不入队。
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
    // ⑤ 窗口内信标已至 / 键已死：撤销登记，按已唤醒处理
    if queued {
        // **跨挂起不得持强引用**：入队成功 ⇒ 队列里那份是权威持有者，本地这份
        // 到此为止。留着它不会影响「正常唤醒」（挂起后帧会恢复、局部量照常 drop），但会
        // 在**被别核 kill 掉**时随栈一起被丢弃——栈没了，引用计数永不回落，被指向的任务
        // 被永久钉住（它的 Team/Space 跟着不 drop，帧与页全留在关机类别账上）。
        drop(task);
    } else {
        void(ticket);
        rise(core::iter::once(task));
    }
    // **挂起前自检**（audit）：此刻本核栈上不该还压着任何"抄件"弱引用 —— 压着就说明
    // 有引用跨过了挂起，而这条调用链一旦被弃，它的 `Drop` 永不执行（见 `weak`）。
    #[cfg(feature = "audit")]
    crate::work::unit::weak::check_block_heldout();
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
/// **形状不变**（裁决）：`park(dur)` 不加参数——键的存活单元是**它自己**（`Alarm`
/// 的「资源」就是那个睡眠者），内部自取一次 `Arc::downgrade`，不让每个调用方各造
/// 一枚弱引用。
///
/// Running → Blocked；返回下一帧 PA（若 scheduler 装了下一 starved）。
pub fn park(duration: Duration) -> usize {
    let Some(task) = current().running_task() else {
        // 唯一调用点（envcall `Park`）恒在任务上下文；退化路径不空转也不挂：
        // 无任务即无「本核无后继」可谈，直接取活。
        return run();
    };
    let me = task.ident.id;
    let wake_at = clock::now().add(duration).as_ticks();
    trace::note(EventKind::Room(RoomEvent::Park {
        tid: me,
        wake_at: wake_at as usize,
    }));
    let life = task.life();
    drop(task);
    match block(WakeKey::Alarm { task: me }, life, duration) {
        Handoff::Switch(pa) => pa,
        // `Alarm` 无投信方，且键的强持有者就是我（我还在跑）⇒ 信标先探不可能命中、
        // 键也不可能已死。
        Handoff::Resume(()) => unreachable!("Alarm 无投信方"),
    }
}

/// 事件等待（`RoomCall::Wait`）：直通 [`block`]。有投信方的键，信标先探可能命中
/// 而当场续跑（[`Handoff::Resume`]）；键已死则 ④ 的锁内判死把它当场放回。
pub fn wait(key: WakeKey, life: Weak<Life>, dur: Duration) -> Handoff<()> {
    block(key, life, dur)
}

// ── 操作：等目标回收（Join） ──

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
///
/// `task.life` = 目标任务的存活单元（弱引用）。调用方（`UnitCall::Join` 入口）本来
/// 就握着目标的 `Arc<Task>`（授权判定要用），交一枚弱引用最自然——**解析在调用方
/// 那一层**，room 不查任务注册表。键的这张站点因此也有了寿命：目标真正消失
/// （`Arc<Task>` 归零）后，残留的空站点会被 `prune` 当场删掉。
///
/// `reaped` = 边界当场读出的「退出钩子已跑完」（`TaskState::Reaped` 由 [`reap`] 独占
/// 置位）。**非法 id 也在边界判掉**（名册点名无此 id ⇒ `Denied`）——判活只此一条来源，
/// 本函数因此**没有失败支**：从前那个 `Err(Denied)` 需要 `target_dead ∧ ¬allocated`
/// 同时成立，而两条来路都蕴含 `allocated`，故它**曾经永远不可达**。
pub fn join(task: TaskLife, reaped: bool, dur: Duration) -> Handoff<bool> {
    if reaped {
        return Handoff::Resume(true);
    }
    if dur == Duration::ZERO {
        return Handoff::Resume(false);
    }
    // 拆壳（`TaskLife` 是"一对"）后**按值移交**存活单元：挂起期间站点是它唯一的
    // 持有者，`join` 这一帧里不留副本（理由同 `block` 头注的"跨挂起不得持强引用"）。
    let TaskLife { id, life } = task;
    match block(WakeKey::Task { id }, life, dur) {
        Handoff::Switch(pa) => Handoff::Switch(pa),
        // 信标已置：目标在「判死 → 入队」的窗口内被回收 ⇒ 当场结论（已回收）。
        Handoff::Resume(()) => Handoff::Resume(true),
    }
}

/// 键退役：放行该键上的**全部**等待者，并把站点**当场删掉**（不留墓碑，也不留空壳）。
///
/// 「目标已回收」与「资源已封印/销毁」是同一件事的两副面孔。这个键再也不会有人投信
/// ——资源侧只在自己退役的那一刻调本函数（`HoleMeta::drop` / `hole::seal` / `bury`），
/// 而 `wipe` 之后资源对象就归零或在归零路上。故「此键已死」这个结论**由 [`Life`]
/// 承担**，不必再靠一张空站点记着：站点值里的 `Weak<Life>` 自己会答，`prune` 的判据
/// 里也已经含了「键已死」这一项。
///
/// **删站点而不是留墓碑**是「站点寿命＝资源寿命」的落地处，也是站点表不随运行增长的
/// 关键：`prune` 只在被调用到**那一个键**上做判定，而 hole id / task id 都单调不复用
/// ⇒ 一个死键的站点若留在表里，此后**再没有任何入口会碰它**。实测（同一个 ELF、同一
/// 台机，追加量按轮计）：只把判据扩成「键已死也算孤儿」而 `wipe` 仍留站点时，追加
/// 6 轮 hole+spawn 让总数 52 → 88（每轮 +6，与追加量成正比）；改成删站点后，追加 8 轮
/// 的总数恒为 0（`sites 0 live 0 tomb 0 orphan 0 waiters 0`，与追加轮数无关）。
///
/// 在飞窗口不受影响：正飞在 `block` ①④ 之间的等待者由 ④ 的锁内判死接住（键在资源
/// 归零后必然判死）；`wipe` 之后再到达的等待者走 ④ 的建立分支重建站点，而那一刻键
/// 要么已死（当场放回）、要么还活（本来就该等）。
///
/// 锁纪律同 [`wake`]：只在站点表（L3）内摘除，锁外 transform + 入队。返回唤醒数。
pub(crate) fn wipe(key: WakeKey) -> usize {
    let waiters = {
        let mut sites = sites(key).lock();
        // 不 `or_insert`、不留信标：站点是「等待者 + 对未来等待者仍有意义的遗留信号」
        // 的容器，键退役后两者都不该留下（信标同样作废——资源侧的 `alive()` 检查已经
        // 拒绝了后来的操作，投信方不存在了）。
        match sites.remove(&key) {
            Some(site) => site.waiters,
            None => VecDeque::new(),
        }
    };
    for w in &waiters {
        void(w.ticket);
    }
    rise(waiters.into_iter().map(|w| w.task))
}
/// 空间退役：删掉该空间名下的**全部**空间键站点，放行它们的等待者。返回唤醒数。
///
/// 与 [`wipe`]（单键）同族，但**触发面不同**：hole 键与 task 键各有自己的退役调用点
/// （`HoleMeta::drop` / `hole::seal` / `bury`），**空间键没有**——空间死掉时没有任何入口
/// 会再碰它的键，而 `prune` 只在"那个键再次被碰到"时才跑 ⇒ 站点永留（实测：关机时
/// `dead 1`，`by kind: space 1`）。
///
/// 调用方 = [`super::reap::bury`]：判定"空间将亡"（唯一强持有者就是这个正在回收的任务）
/// 之后调。遍历全部分片、**逐片取放**（绝不持跨片锁）；摘出的等待者与键一起退役
/// （`void(ticket)` 消音到点 + `rise` 放回就绪）。
pub(crate) fn wipe_space(space: usize) -> usize {
    let mut woken = 0usize;
    for shard in 0..SITE_SHARDS {
        // 作用域即临界区：锁内只取、锁外 drop（`Arc<Task>` 的 drop 链会取 L2）。
        let taken: Vec<Waiter> = {
            let mut sites = shard_at(shard).lock();
            let keys: Vec<WakeKey> = sites
                .keys()
                .filter(|k| matches!(k, WakeKey::Space { space: s, .. } if *s == space))
                .copied()
                .collect();
            let mut out = Vec::new();
            for key in keys {
                if let Some(site) = sites.remove(&key) {
                    out.extend(site.waiters);
                }
            }
            out
        };
        for w in &taken {
            void(w.ticket);
        }
        woken += rise(taken.into_iter().map(|w| w.task));
    }
    woken
}

// ── 操作：唤醒 ──

/// 叫醒一个：摘队首 → 放回就绪。无人在等 → 置信标（防漏唤醒）。返回是否唤到人。
///
/// **键已死 ⇒ `false` 且不建站点**（A2 裁决）：资源没了，这个键再也不会有等待者，
/// 给它留站点或信标都是墓碑的另一种叫法。此处顺带把死键的残留站点删掉——
/// 死键的队列必然空（能入队 ⇒ 那时键还活着），故直接 `remove` 是安全的。
///
/// 消费方 = envcall 与 mail 的投信方；跨核经 steal 再平衡（同 [`redeem`]）。
///
/// **信标可能陈旧**：「信号」与「数据」是两份状态——等待者后来直接取走数据
/// （裸 pull 成功，不经 `wait`）时信标不被消费，下一次 `wait` 就立刻返回「已唤醒」
/// 而实际无数据。故 `wait` 的返回**只是提示**，调用方必须自己复核条件
/// （`hole::wait` 已复核就绪位；有界等待方还须按 deadline 循环，见
/// `docs/dispatch.md` §11.4）。
pub fn wake(key: WakeKey, life: &Weak<Life>) -> bool {
    let popped = {
        let mut sites = sites(key).lock();
        if Life::dead(life) {
            sites.remove(&key);
            None
        } else {
            let site = sites.entry(key).or_insert_with(|| Site::new(life));
            let popped = match site.waiters.pop_front() {
                Some(w) => Some(w),
                None => {
                    site.pend = true;
                    None
                }
            };
            prune(&mut sites, key);
            popped
        }
    };
    let Some(w) = popped else { return false };
    void(w.ticket);
    rise(core::iter::once(w.task));
    true
}

/// 到期兑现：`chrono` 交回的不透明句柄，在这里还原成「谁」。
///
/// 一段走完，不再认识 park / wait / join 的区别：
///   票根 → 取回（键 + 持票人）→ 从该键的队列里按票号摘出。
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
        // 票号即到点登记的身份：作废票根并取回「在哪个键上等 + 持票人」（已回收 → 落空）。
        // **键取自票根而不是任务 payload**：本路径是观察者（那份持票人 Arc 是临时的，
        // 任务随时可能被别核放行），读 payload 就是读一个被独占写的字段。
        let Some((key, holder)) = void(Ticket(handle)) else {
            continue;
        };
        drop(holder); // 队列里的那份才是权威强持有者（票根只存 Weak）
        // 从该键的队列里摘出**这一票**的等待者（票号对不上 = 陈旧，落空）。
        let popped = {
            let mut sites = sites(key).lock();
            let w = sites.get_mut(&key).and_then(|site| {
                site.waiters
                    .iter()
                    .position(|w| w.ticket == Ticket(handle))
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
