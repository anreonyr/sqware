// 等待机（wait）——「挂起 → 唤醒」这条链对外的几个入口：park / wait / join / wake /
// wipe / redeem，共用一条挂起实现 `block`。
//
// 站点表（唯一容器）与票根分居 `site` / `holder`；这两个子模块里跨到 `messenger`
// 一级的条目取 `pub(in super::super)`——刚好到 `messenger`，不放宽到 `pub(crate)`。

pub(super) mod holder;
pub(super) mod site;

use alloc::sync::{Arc, Weak};
use core::time::Duration;

use crate::runtime::chrono::{clock, timer};
use crate::runtime::diagnose::trace::{self, EventKind, RoomEvent};
use crate::work::room::conductor;
use crate::work::room::scheduler::core::{current, kick};
use crate::work::room::scheduler::trap::run;
use crate::work::unit::gate::GateError;
use crate::work::unit::life::{Life, TaskLife};
use crate::work::unit::task::{Task, TaskState};

use self::holder::{Ticket, hold, void};
use self::site::{Fwd, SITE_SHARDS, Site, WakeKey, prune, shard_at, sites, take_beacon};
use super::handoff::Handoff;

// ── 操作：挂起（用 scheduler::core::Scheduler::swap） ──

/// 挂起的唯一实现：三处入口（`park` / `wait` / `join`）只差一个键。
///
/// 时序（两个竞态闭合点）：
///   ① 信标先探——信号已至 → 不挂起（不碰站点表：缺键即无信标）
///   ② 备料 + 登记——**站点就位（唯一的分配点）** / 票根 / 到点，全部在离核之前：
///      票根先于 `tock`（**堆可见 ⇒ 票根必在**），而两步都要能答错
///   ③ 离核——借 scheduler 跨边界原语把 running 卸下（槽位 settled）
///   ④ 入链——写等待点 + 接到站点链尾（**零分配**）；**锁内先判键死活、再查一次信标**
///   ⑤ 窗口内信标已至 / 键已死 / 站点已被删 → 撤销登记，按「已唤醒」处理（Starved 入队）
///
/// `life` = 本键的存活单元（弱引用，调用方随键一起交进来——room 不查任何注册表）。
/// ④ 的锁内判死就是 A2 说的「关上在飞窗口」：一个正飞在 ①④ 之间的等待者，此前
/// 只能靠 `wipe` 留下的墓碑接住；现在键自己会答（`weak.upgrade` 失败），于是墓碑
/// 可以不留。键已死这一支**走既有回滚**（⑤ 的 `void(ticket)` + `rise`），不新增
/// 任何清理机制——`Blocked` 只在 push 那一支被写，状态仍与容器一致。
///
/// # ② 为什么必须在离核之前（失败域）
///
/// 挂起没有失败域可挂（`Handoff` 两态里没有"失败"），所以唯一会分配的一步按仓内
/// 惯例提到装配之前；而"之前"的边界是 **`current().swap()`**——离核之后本核就没
/// 有自己的任务了，此时再想返回只能让核上空转（实测：下一次 envcall 直接
/// `envcall without running task`）。失败时一个字都没欠：站点已就位、票根与
/// 到点未登记、任务状态未改，调用方当场拿到 `OoM`。
///
/// 锁纪律：站点表与票根都是 L3，**绝不互相嵌套**——「作用域内取、作用域外用」。
fn block(key: WakeKey, life: Weak<Life>, dur: Duration) -> Result<Handoff<()>, GateError> {
    // ① 信标先探
    if take_beacon(key) {
        return Ok(Handoff::Resume(()));
    }
    // ② 备料 + 登记（**全部在离核之前**）
    //
    // 挂起这条路上没有失败域（`Handoff` 两态里没有"失败"），故按仓内惯例把唯一会
    // 分配的一步提到装配之前。**必须在 `swap()` 之前**：`swap()` 已经把本任务换下
    // 核，之后返回等于让本核没有任务（实测：下一次 envcall 直接
    // `envcall without running task` panic）。失败时一个字都没欠——站点已就位、
    // 票根与到点未登记、任务状态未改、本核仍持着自己的任务，`OoM` 当场交给调用方，
    // 它可能马上重试。
    //
    // 备料三件事：**站点就位**（唯一会分配的一步）、票根、到点。站点是 ④ 的落脚处
    // ——链挂在站点上；它同时是 ④ 只 `get_mut` 的前提（见 ④）。`life` 按值移进站点
    // （跨挂起不留副本），且它带的就是**本键**的存活单元：同一个键只有一份 `Life`，
    // 故这枚弱引用与入口无关（wait / join 指同一个分配），赋值不是「换主」而是
    // 「同一事实的重写」。
    let Some(me) = current().running_task() else {
        // envcall 恒在任务上下文（见 `dispatch` 头注）；退化路径不挂起、不动表，
        // 按"条件未就绪"答（`Busy`）。
        return Err(GateError::Busy);
    };
    // **离核前自查**：我正在离开核——若此刻已被点名（他杀 / 级联的跨核分支），就地自退。
    //
    // 为什么落在这里：`doomed` 的兑现必须发生在**任务自己的时刻**。投那一记 SSIP 是"一次
    // 投递"，它可能被别的上下文取走（见 `doom::doomed_nudge`），而**一个已经挂起的任务
    // 永远不是"本核当前任务"** ⇒ 它再也等不到第二次机会，那一笔就成了孤儿。挂起的入口
    // 只有本函数，故这一处就是"它要睡了"那个时刻。
    //
    // 位置在**备料之前**：自退不欠任何登记（站点 / 票根 / 到点）一个字都没写。
    if let Some(reason) = super::take_doomed(me.ident.id) {
        drop(me);
        super::set_exit_reason(reason);
        // 自退：此刻本核仍然持着我（未 `swap`），与 `Reap` / SSIP 那两处同一形状。
        return Ok(Handoff::Switch(super::quit()));
    }
    {
        let mut sites = sites(key).lock();
        if !sites.contains_key(&key) {
            sites.try_reserve(1).map_err(|_| GateError::OoM)?;
            let mut site = Site::new(&Weak::new());
            site.life = life;
            sites.insert(key, site);
        }
    }
    let ticket = Ticket::alloc();
    let at = (dur != Duration::MAX).then(|| clock::now().add(dur).as_ticks());
    if let Some(at) = at {
        hold(ticket, key, &me).map_err(|()| GateError::OoM)?;
        if timer::tock(ticket.raw(), at).is_err() {
            void(ticket); // 到点没登记上 ⇒ 票根也不留（`void` 顺带消音，幂等）
            return Err(GateError::OoM);
        }
        // **本次改动唯一的新增行为**：到点登记成功后，把**本核**武装点收到
        // `min(失明上限, 最近活到点)`。登记了却不武装 = 到点没人兑现——这正是病根：
        // 四核全忙时 `Park{millis}` 晚一拍（失明上限）、有空闲核时精确，同一调用差 20 倍。
        //
        // 为什么**不需要 IPI**：`redeem`/`drain` 是**全局**的（堆全局一份、一次取走全部
        // 到期项），登记这颗核把自己叫醒就够——证人只需要一个，而登记者就是。
        //
        // 照实记（量过）：正因为 `redeem` 全局，**任何一颗空闲核**都会按 `due()` 武装并
        // 兑现全部到点 ⇒ 树内 workload（soak 全员 1ms 轮询睡眠、rig 只有一枚 churn）里总有核
        // 空闲，这条债**量不出来**（release soak 16k 样本 A/B 无差别）。要量它得把"空闲核"
        // 从机器上删掉：单核 + 占核者 + 一枚**稀疏**打点者
        // （`QEMU_SMP=1 scripts/load.sh 1 --release`）。实测（n=81，同一台子只差那三处武装
        // 式子）：**修复前 `late_avg=97 ms / late_max=99 ms`；修复后 `0 ms / 125 µs`**；陷阱数
        // 645 两边一致 ⇒ 没多花中断。
        //
        // 也是那次实测发现：**这一句在本核里是冗余的**——停下一枚任务后，那颗核必然会再武装
        // 一次（`seat` 下一枚 → `hart.rs`，或进空闲 → `fetch.rs`），所以只把这一句关掉，忙机台
        // 上的迟到仍是 0。留着是**保险带**：让不变量在"登记那一刻"就成立，而不是依赖"这颗核
        // 稍后一定会再武装一次"；删掉则少一次 SBI ecall、少一处改动——**留/删待裁**。
        //
        // 位置必须在 `tock` 之后、**锁外**：`tock` 内部持 `TIMER_HEAP` 锁，SBI ecall
        // 不许进临界区——故禁止把这次武装塞进 `tock`。
        timer::beat_until(timer::blind_ceiling());
    }
    // 强引用到此为止：**跨挂起不得持强引用**（`me` 只是登记用的临时量）。
    drop(me);
    // ③ 离核
    let (mut task, next_pa) = current().swap();
    trace::note(EventKind::Room(RoomEvent::Wait {
        tid: task.ident.id,
        // 诊断用折叠值：键成枚举后不再有「人可读的位打包」形态。
        key: key.fold() as usize,
    }));
    // ④ 写等待点 + 入链（锁内判死活 + 查信标）
    Task::exclusive(&mut task).transform(TaskState::Blocked {
        key,
        ticket,
        next: None,
    });
    let queued = {
        let mut sites = sites(key).lock();
        // **只 `get_mut`，不 `entry().or_insert_with()`**：站点是 ② 就位好的（唯一分配
        // 点在那一侧），这里再要一次分片表容量就等于把失败的入口搬到了 `swap()` 之后
        // ——那一侧没有失败域。站点不在 = 它在 ② 与此刻之间被别的核删了（`prune` /
        // `wake` / `wipe` 都能删），三支一起按「已唤醒」收尾。
        let queued = match sites.get_mut(&key) {
            None => false,
            // 键已死（资源没了）：不入链、也不留站点——死键的链必然空（能入链 ⇒ 入链那
            // 一刻键还活着），故下面的 `prune` 会当场把这个空壳删掉。
            Some(site) if Life::dead(&site.life) => false,
            // 窗口内信标已至：消费它，不入链。
            Some(site) if site.pend => {
                site.pend = false;
                false
            }
            // 入链 = 两次指针写：链尾的 `next` 指向我，我成为新链尾。
            Some(site) => {
                site.push_back(task.clone());
                true
            }
        };
        // 撤销阻塞那一支没有留下等待者：空壳站点随手删掉（判据见 `prune`——链空、
        // 无信标**且无转发登记**才算空壳；只挂着转发的那一类有语义，要留下）。
        prune(&mut sites, key);
        queued
    };

    // ⑤ 窗口内信标已至 / 键已死 / 站点已被删：撤销登记，按已唤醒处理
    if queued {
        // **跨挂起不得持强引用**：入链成功 ⇒ 链上那份是权威持有者，本地这份
        // 到此为止。留着它不会影响「正常唤醒」（挂起后帧会恢复、局部量照常 drop），但会
        // 在**被别核 kill 掉**时随栈一起被丢弃——栈没了，引用计数永不回落，被指向的任务
        // 被永久钉住（它的 Team/Space 跟着不 drop，帧与页全留在关机类别账上）。
        drop(task);
    } else {
        void(ticket);
        rise(core::iter::once(task));
    }
    // **挂起前自检**（framework）：此刻本核栈上不该还压着任何"抄件"弱引用 —— 压着就说明
    // 有引用跨过了挂起，而这条调用链一旦被弃，它的 `Drop` 永不执行（见 `weak`）。
    #[cfg(feature = "framework")]
    crate::work::unit::weak::check_block_heldout();
    // 本核无后继即就地取活：`run()` 只会循环到有帧或停机，故落点恒为 `Switch`。
    Ok(Handoff::Switch(next_pa.unwrap_or_else(run)))
}

/// 放回就绪——「唤醒」的全部效果就是这一件事。
///
/// `wake` / `wipe` / `redeem` 与撤销阻塞四条路径的收尾完全同形（置 Starved →
/// 记事件 → 踢到 [`pick`](conductor::pick) 挑中的那颗核），故只写一遍。返回唤醒数。
///
/// 逐枚 `kick(pick(), t)`（甲案）：游标自然轮转 ⇒ 一批活摊到多颗核上，而不是全堆在
/// 唤醒者自己的队列里等它 yield；落点核正等着就顺手被叫醒。**入队与唤醒同点**是这条
/// 路径的要点——旧形状"推本核队列 + 批量后踢一次"把"谁持有活"与"谁被叫醒"分开了，
/// 而兜底（源核下次 yield 自取）在 S 态域任务上不成立（空转不吃陷阱 ⇒ 永不 yield）。
///
/// 批量不再能省成一次 IPI：落点核每枚都可能不同（游标轮转），一记 IPI 只能叫醒一颗核
/// ——省下来就会把活留在别的核的队列里。真全忙时 `fallback` 记这一笔，活靠落点核下次
/// 进 `fetch` 自取（那是本路径**仅剩**的兜底）。
fn rise<I: IntoIterator<Item = Arc<Task>>>(tasks: I) -> usize {
    let mut woke = 0;
    for task in tasks {
        let mut t = task;
        Task::exclusive(&mut t).transform(TaskState::Starved { next: None });
        trace::note(EventKind::Room(RoomEvent::Wake { tid: t.ident.id }));
        kick(conductor::pick(), t);
        woke += 1;
    }
    woke
}

/// 拆一条 `Blocked` 等待链：每次吐一环，**吐之前先作废它的到点登记**（`void` 幂等），
/// 并就地摘掉那一环（离开 `Blocked` 必须摘链，`transform` 的断言就立在这上面）。
///
/// 逐环摘而不是整条一次 drop：一次性 drop 会把长链压进调用栈（与 `starved_clear`
/// 同理）。交给 [`rise`] 当迭代器用——甲案之后 `rise` 是**逐枚** `kick`（落点核游标
/// 轮转），故这里每吐一环就换一颗核，不再有"整批一次踢"那回事。链头进门时已经出了
/// 分片锁——本迭代器不上锁、不分配。
struct Unchain {
    cur: Option<Arc<Task>>,
}

impl Iterator for Unchain {
    type Item = Arc<Task>;

    fn next(&mut self) -> Option<Arc<Task>> {
        let mut task = self.cur.take()?;
        void(Task::blocked_ticket(&mut task));
        self.cur = Task::blocked_next(&mut task).take();
        Some(task)
    }
}

/// 纯睡（`RoomCall::Park`）：键是 `Alarm { 我 }`——无人投信，只有期限会响。
///
/// **形状不变**（裁决）：`park(dur)` 不加参数——键的存活单元是**它自己**（`Alarm`
/// 的「资源」就是那个睡眠者），内部自取一次 `Arc::downgrade`，不让每个调用方各造
/// 一枚弱引用。
///
/// Running → Blocked；返回下一帧 PA（若 scheduler 装了下一 starved）。
pub fn park(duration: Duration) -> Result<usize, GateError> {
    let Some(task) = current().running_task() else {
        // 唯一调用点（envcall `Park`）恒在任务上下文；退化路径不空转也不挂：
        // 无任务即无「本核无后继」可谈，直接取活。
        return Ok(run());
    };
    let me = task.ident.id;
    let wake_at = clock::now().add(duration).as_ticks();
    trace::note(EventKind::Room(RoomEvent::Park {
        tid: me,
        wake_at: wake_at as usize,
    }));
    let life = task.life();
    drop(task);
    match block(WakeKey::Alarm { task: me }, life, duration)? {
        Handoff::Switch(pa) => Ok(pa),
        // `Alarm` 无投信方，且键的强持有者就是我（我还在跑）⇒ 信标先探不可能命中、
        // 键也不可能已死。
        Handoff::Resume(()) => unreachable!("Alarm 无投信方"),
    }
}

/// 事件等待（`RoomCall::Wait`）：直通 [`block`]。有投信方的键，信标先探可能命中
/// 而当场续跑（[`Handoff::Resume`]）；键已死则 ⑤ 的锁内判死把它当场放回。
pub fn wait(key: WakeKey, life: Weak<Life>, dur: Duration) -> Result<Handoff<()>, GateError> {
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
/// 等"**我自己这张权限表**里落进一枚"（`UnitCall::Fall` 的落点）。
///
/// `me` 必须是**调用者自己**的 `TaskLife`——适配层从 `current().running_task()` 取，
/// 不由参数给：等的是谁的表现在根本没有填的地方。
///
/// `Resume(true)` = 取到了信标（自上次取走以来落过表）；**不保证"就是我等的那一枚"**
/// ——醒来自己扫表分辨。与 [`join`] 的唯一差别是 `dur == ZERO` **不特判**：`join` 探的是
/// 资源状态（重复问答案一样，故不消费），这里探的是**事件位**——问了就是取了，
/// 不取就会永远答"是"（`block` 第一步的 `take_beacon` 正好是这件事）。
pub fn fall(me: TaskLife, dur: Duration) -> Result<Handoff<bool>, GateError> {
    let TaskLife { id, life } = me;
    match block(WakeKey::Pies { task: id }, life, dur)? {
        Handoff::Switch(pa) => Ok(Handoff::Switch(pa)),
        Handoff::Resume(()) => Ok(Handoff::Resume(true)),
    }
}

pub fn join(task: TaskLife, reaped: bool, dur: Duration) -> Result<Handoff<bool>, GateError> {
    if reaped {
        return Ok(Handoff::Resume(true));
    }
    if dur == Duration::ZERO {
        return Ok(Handoff::Resume(false));
    }
    // 拆壳（`TaskLife` 是"一对"）后**按值移交**存活单元：挂起期间站点是它唯一的
    // 持有者，`join` 这一帧里不留副本（理由同 `block` 头注的"跨挂起不得持强引用"）。
    let TaskLife { id, life } = task;
    match block(WakeKey::Task { id }, life, dur)? {
        Handoff::Switch(pa) => Ok(Handoff::Switch(pa)),
        // 信标已置：目标在「判死 → 入队」的窗口内被回收 ⇒ 当场结论（已回收）。
        Handoff::Resume(()) => Ok(Handoff::Resume(true)),
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
/// 的总数恒为 0（当时的审计探针打出 `sites 0 live 0 tomb 0 orphan 0`，与追加轮数无关；
/// 那枚探针已不在树里，四类形态的口径见 `site.rs` 的 `prune`）。
///
/// 在飞窗口不受影响：正飞在 `block` ①④ 之间的等待者由 ④ 的锁内判死接住（键在资源
/// 归零后必然判死）；`wipe` 之后再到达的等待者由 ② 把站点建回来，而那一刻键要么已死
/// （当场放回）、要么还活（本来就该等）。
///
/// 锁纪律同 [`wake`]：只在站点表（L3）内摘除，锁外 transform + 入队。返回唤醒数。
pub(crate) fn wipe(key: WakeKey) -> usize {
    let chain = {
        let mut sites = sites(key).lock();
        // 不 `or_insert`、不留信标：站点是「等待者 + 对未来等待者仍有意义的遗留信号」
        // 的容器，键退役后两者都不该留下（信标同样作废——资源侧的 `alive()` 检查已经
        // 拒绝了后来的操作，投信方不存在了）。
        sites.remove(&key)
    };
    // 成员键退役（资源封印/销毁）：**先**叫醒等着这些成员的组（它们醒来会按
    // `cells()` 快照复核，死的格子自然不在里面），再放行本键自己的等待者。
    let chain = match chain {
        Some(site) => {
            for (id, life) in site.fwd.entries() {
                knock(WakeKey::Tole { id }, life);
            }
            site.head
        }
        None => None,
    };
    rise(Unchain { cur: chain })
}
/// 叫醒一个键上的等待者；**站点不在就替它建一枚只带信标的站点**（寿命边由调用方给）。
///
/// 用在转发那一跳。那一跳**不许落空**：成员推可能早于等组的人入 `block`（组站点还
/// 不存在），此时若什么都不做，这一条唤醒就丢到期限为止——"等 N 个源"的语义当场破掉。
/// 故这里带上了目标的存活单元：建出来的站点随目标一起作废（`prune` 的既有判据），
/// 不留墓碑。目标**已经死了**才什么都不做。
///
/// 站点在而队列空 ⇒ 置信标：那是"等待者正在 ①④ 之间飞"的窗口（它在 ④ 会消费掉
/// 这一位，当场返回去复核），也是本函数**不丢唤醒**的另一半。
fn knock(key: WakeKey, life: &Weak<Life>) -> usize {
    let popped = {
        let mut sites = sites(key).lock();
        let popped = match sites.get_mut(&key) {
            Some(site) => match site.pop_front() {
                Some(task) => Some(task),
                None => {
                    site.pend = true;
                    None
                }
            },
            // **站点还不存在**（等组的人还没走到 `block`）：替它建一枚**只带信标**的站点。
            // 这一跳不许落空——成员推早于等待者入 `block` 时若什么都不做，那条唤醒就丢到
            // 期限为止。寿命边用**目标**的存活单元：目标死了这枚站点随 `prune` 走，不留墓碑。
            None => {
                if !Life::dead(life) && sites.try_reserve(1).is_ok() {
                    let mut site = Site::new(life);
                    site.pend = true;
                    sites.insert(key, site);
                }
                None
            }
        };
        prune(&mut sites, key);
        popped
    };
    let Some(mut task) = popped else { return 0 };
    void(Task::blocked_ticket(&mut task));
    rise(core::iter::once(task))
}

/// 登记转发：投信 `key` 时也认醒组 `tole`。站点不在就建一个（**唯一的分配点**，
/// 备不出容量返 `Err`——登记侧有失败域，与唤醒侧的无失败通道正好相对）。
///
/// 幂等：重复登记同一个组不叠加（站点满了返 `Err`，由调用方报 `OoM`，不静默丢）。
///
/// **建出来的站点不会当场被 `prune` 收走**：`fwd` 自己就是判据的一项（见 [`prune`]
/// 头注第三项）。这一条不是修辞——成员键上没有等待者、也没有信标是常态（等组的人等
/// 的是**组自己的键**），少了它，末尾那次 `prune` 会把刚写下的登记连同站点一起删掉，
/// 而本函数**照样返 `Ok`**：调用方（`tole::hang`）以为登记成功，投信那一侧却再也叫
/// 不醒这个组（实测：`await_(usize::MAX)` 的板线程永远不醒，同一段代码改成毫秒轮询
/// 就好——轮询的唤醒来自到点，不经这条转发）。
pub(crate) fn forward(
    key: WakeKey,
    life: Weak<Life>,
    tole: usize,
    tole_life: Weak<Life>,
) -> Result<(), ()> {
    let mut sites = sites(key).lock();
    if !sites.contains_key(&key) {
        sites.try_reserve(1).map_err(|_| ())?;
        sites.insert(key, Site::new(&life));
    }
    let site = sites.get_mut(&key).ok_or(())?;
    let r = site.fwd.attach(tole, tole_life);
    prune(&mut sites, key);
    r
}

/// 撤销转发：投信 `key` 时不再认醒组 `tole`；没登记过即无事。站点不在即无事。
///
/// 摘掉最后一格之后，那个「只为转发而存在」的站点（链空、无信标）被 `prune` 当场
/// 收走——登记的寿命与站点的寿命因此精确对齐，不留"空转发"的壳。
pub(crate) fn unforward(key: WakeKey, tole: usize) {
    let mut sites = sites(key).lock();
    if let Some(site) = sites.get_mut(&key) {
        site.fwd.detach(tole);
    }
    prune(&mut sites, key);
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
        // **一次一个站点**：锁内只摘、锁外处理（`Arc<Task>` 的 drop 链会取 L2），而
        // **等待链随站点本身一起出锁**（链头链尾两份强引用都在 `Site` 里）⇒ 全程零分配。
        // 旧版在片内先 `keys().collect()` 再开一个 `Vec` 收等待者——两笔都发生在
        // **持锁期间**，而收尾路径（`bury`）没有失败域，内存吃紧就是一次整机 halt。
        loop {
            let taken = {
                let mut sites = shard_at(shard).lock();
                let key = sites
                    .keys()
                    .find(|k| matches!(k, WakeKey::Space { space: s, .. } if *s == space))
                    .copied();
                key.and_then(|key| sites.remove(&key))
            };
            let Some(site) = taken else { break };
            // 空链的站点（只剩信标）也照删——键跟着空间一起退役，信标作废。
            woken += rise(Unchain { cur: site.head });
        }
    }
    woken
}

// ── 操作：唤醒 ──

/// 叫醒一个：摘链头 → 放回就绪。无人在等 → 置信标（防漏唤醒）。返回是否唤到人。
///
/// **键已死 ⇒ `false` 且不建站点**（A2 裁决）：资源没了，这个键再也不会有等待者，
/// 给它留站点或信标都是墓碑的另一种叫法。此处顺带把死键的残留站点删掉——
/// 死键的等待链必然空（能入链 ⇒ 那时键还活着），故直接 `remove` 是安全的。
///
/// 站点不在（无人等过这个键）时才需要**建**一个来承载信标，那是本函数唯一的分配点，
/// 故**锁内先备后插**（同一把分片锁保证中间没人抢走那格容量）。备不出来就**丢掉这枚
/// 信标**（本函数返回 `false`）——本函数没有失败通道（投信方只看"叫到人没有"），而信标
/// 本来就只是提示（见下），丢它 = 少一次"当场返回"，不是少一次唤醒。于是这条路上也
/// 没有"要么扩容要么 halt"。
///
/// 消费方 = envcall 与 mail 的投信方；跨核经 steal 再平衡（同 [`redeem`]）。
///
/// **信标可能陈旧**：「信号」与「数据」是两份状态——等待者后来直接取走数据
/// （裸 pull 成功，不经 `wait`）时信标不被消费，下一次 `wait` 就立刻返回「已唤醒」
/// 而实际无数据。故 `wait` 的返回**只是提示**，调用方必须自己复核条件
/// （`hole::wait` 已复核就绪位；有界等待方还须按 deadline 循环）。
pub fn wake(key: WakeKey, life: &Weak<Life>) -> bool {
    let mut fwd = Fwd::empty();
    let popped = {
        let mut sites = sites(key).lock();
        if Life::dead(life) {
            sites.remove(&key);
            None
        } else {
            // 摘链头；站点不在 ⇒ 记下来，出借后再建（建它要 `sites` 的 &mut）。
            // `site` 的借用在块内结束——`prune` 还要一次 `&mut sites`。
            let mut beacon_only = false;
            let popped = match sites.get_mut(&key) {
                Some(site) => match site.pop_front() {
                    Some(task) => Some(task),
                    None => {
                        site.pend = true;
                        None
                    }
                },
                None => {
                    beacon_only = true;
                    None
                }
            };
            if beacon_only && sites.try_reserve(1).is_ok() {
                let mut site = Site::new(life);
                site.pend = true;
                sites.insert(key, site);
            }
            // **转发格拷出来**（`Fwd` 是定长 `Copy`）：投信本键时也要叫醒那些组，
            // 但拿别的分片锁必须在放开本片之后（逐片取放，绝不嵌套）。
            fwd = sites.get(&key).map_or_else(Fwd::empty, |s| s.fwd.clone());
            prune(&mut sites, key);
            popped
        }
    };
    // 叫醒转发目标（组站点）：**不许落空**——站点不在就替它建一枚只带信标的（见 `knock`）。
    for (id, life) in fwd.entries() {
        knock(WakeKey::Tole { id }, life);
    }
    let Some(mut task) = popped else { return false };
    // 票在摘链那一刻从载荷里读（持分片锁时读得到；见 `Task::blocked_ticket`）。
    void(Task::blocked_ticket(&mut task));
    rise(core::iter::once(task));
    true
}

/// 到期兑现：`chrono` 交回的不透明句柄，在这里还原成「谁」。
///
/// 一段走完，不再认识 park / wait / join 的区别：
///   票根 → 取回（键 + 持票人）→ 从该键的等待链里按票号摘出。
/// 陈旧的登记在每一步都自然落空（票根已被 `void`、任务已不阻塞、票号对不上），
/// 故不需要任何「取消」记账。
///
/// 按 tock 堆取到期者（与入队顺序无关）；堆锁先放后取，绝不持堆锁取调度锁
/// （防 ABBA）。返回：本次是否撤出过任务（空闲核的哑睡壳判定用）。
/// 由 trap 路径（S-timer 处理）与空闲核归队时在本 hart 触发。
pub fn redeem() -> bool {
    // 两块**栈上**固定缓冲：句柄一块、放行任务一块。批量收集再统一 `rise`——`rise` 里
    // 逐枚 `kick(pick(), …)`（甲案：落点核由游标轮转，故批量不再能省成一次 IPI，见 `rise`）；
    // 两块都由本帧出，故这条路上没有分配。
    const MAX_DUE: usize = 64;
    let mut due = [0u64; MAX_DUE];
    let n = timer::drain(clock::now(), &mut due);
    let mut tasks: [Option<Arc<Task>>; MAX_DUE] = [const { None }; MAX_DUE];
    let mut woken = 0usize;
    for (slot, &handle) in tasks.iter_mut().zip(&due[..n]) {
        // 票号即到点登记的身份：作废票根并取回「在哪个键上等 + 持票人」（已回收 → 落空）。
        // **键取自票根而不是任务 payload**：本路径是观察者（那份持票人 Arc 是临时的，
        // 任务随时可能被别核放行），读 payload 就是读一个被独占写的字段。
        let Some((key, holder)) = void(Ticket(handle)) else {
            continue;
        };
        drop(holder); // 队列里的那份才是权威强持有者（票根只存 Weak）
        // 从该键的等待链里摘出**这一票**的那一环（票号对不上 = 陈旧，落空）。摘的
        // 动作是「走链找 + 接前驱」，全在分片锁内完成（与 `doom::pop_waiter` 同一手法，
        // 只是它按身份找、这里按票找）。
        let popped = {
            let mut sites = sites(key).lock();
            let pick = &mut |t: &mut Arc<Task>| Task::blocked_ticket(t) == Ticket(handle);
            let w = sites.get_mut(&key).and_then(|site| site.remove_if(pick));
            prune(&mut sites, key);
            w
        };
        let Some(w) = popped else { continue };
        *slot = Some(w);
        woken += 1;
    }
    let _ = woken;
    rise(tasks.iter_mut().filter_map(Option::take)) > 0
}
