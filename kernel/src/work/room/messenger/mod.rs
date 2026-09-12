// 事件队列（messenger）— 任务不在 running 槽时的状态机。
//
// 任务离开 running 槽有三种过渡：park（纯睡）、wait（按唤醒源等信号）、reap（退出）。
// 前两种与「等目标回收」现在共用一条挂起实现 `block`——它们只差一个键。
// 三种都借 [`scheduler::core::Scheduler::swap`] 跨边界原语把
// running 卸下（槽位 settled：装下一 starved 或降级 Last），再挂进站点表。
// 恢复路径三条：[`wake`]（信号到，一个）、[`wipe`]（键退役，全部）、
// [`redeem`]（到点，按票），都把任务转 Starved 推回本核 + kick。
//
// 簿记：sites（唤醒源 → 信标 + 等待者队列）、holders（票 → 持票人）、husks
// （Arc<Task> 队列）。**两张等待表分工**：站点表是挂起任务的唯一容器（谁在等、
// 等什么）；票根只回答「这一票是谁」（到点时凭票认领）。「等什么」不复制进票根
// ——从持票人自己那张票上读（`TaskState::Blocked`）。旧版的 parked /
// wait_times / join_times 三张旁路表因此消失。
// 锁序：**L1（调度器）与 L3（本域各表）任何方向都不得嵌套**——持任一 L3 期间
// 不得调用 scheduler 的任何加锁方法，也不得在锁内 drop `Arc<Task>`（drop 链会
// 取 Space 锁 L2）。各路径的写法统一为「作用域内取、作用域外用」：block/wake/
// wipe/redeem 在块内摘出 Waiter、块外 push 回 scheduler；bury 块内出队、
// 块外回收；rip 块内 take 整表、块外 drop。
//
// 反向耦合清零：dock / ring 的 task_exit 反向耦合走两步拆——step 5 引入 exit
// hook 注册面后，bury 不再硬编码子系统名。本 step 暂留直调作为过渡。

mod doom;
mod handoff;
mod reap;
mod wait;

// `doom` / `reap` 按 `super::{...}` 取站点表与票根，5a 拆出的这两个面本轮不动——
// 故它们要的三个名字在父模块留一份私有 `use`（可见范围与拆分前一致：只到本域）。
use doom::doomed;
use reap::HUSKS;
use wait::holder::{holders, void};
use wait::site::{SITE_SHARDS, prune, shard_at};

/// 退场原因码的**逐核暂存槽**：`Reap` / 故障隔离杀在调 `quit` 之前写，`quit` 读它
/// 并写进 `RoomEvent::Exit`，随后清零（下一次退场重新写）。
///
/// 为什么不挂在 `Task` 上：故障隔离那两条路径**此刻已经离核**（`current()` 返回
/// `None`），拿不到 `Arc<Task>` 去经 `exclusive` 改字段；而按 tid 回名册反查会多出
/// 一条可能不一致的路径。原因是**每核一次退场**的瞬态数据，放核内槽最贴合它的寿命。
///
/// 槽与核一一对应：一次退场的全程（写 → `quit` → 读）在同一次 trap 处理里完成，
/// 不跨核、不跨任务。
static EXIT_REASON: [core::sync::atomic::AtomicUsize; crate::machine::MAX_HART_SLOTS] =
    [const { core::sync::atomic::AtomicUsize::new(0) }; crate::machine::MAX_HART_SLOTS];

/// 记下本次退场的原因码（见 [`EXIT_REASON`]）。
pub(crate) fn set_exit_reason(reason: usize) {
    let slot = crate::machine::hart_id().min(crate::machine::MAX_HART_SLOTS - 1);
    EXIT_REASON[slot].store(reason, core::sync::atomic::Ordering::Relaxed);
}

/// 取出并清零本次退场的原因码（`quit` 用）。
fn take_exit_reason() -> usize {
    let slot = crate::machine::hart_id().min(crate::machine::MAX_HART_SLOTS - 1);
    EXIT_REASON[slot].swap(0, core::sync::atomic::Ordering::Relaxed)
}

// ── 内核给的退出原因码 ──
//
// 与 `RoomCall::Reap` 带上来的"域自己的诊断编号"共用同一个字段，值域不重叠：域从 1
// 开始编号，内核用高位段。三枚码与 [`EXIT_REASON`] 的槽住在一起——**码的账在持有槽的
// 地方**（一个事实一份账：谁写这格，谁登记它的取值）。
//
// 他杀与级联两条路都**不在这颗核上**发生（受害者是在别的核上被 IPI 唤起、自己在
// `trap.rs` 里自退的），故它们的码随杀令躺在 `doomed` 集合里，由受害者那颗核取出来
// 写进**自己**的槽。

/// 故障隔离杀（不可解析的缺页 / 其它用户异常）。
pub(crate) const EXIT_FAULT: usize = 0xFFFF_FFFF;

/// 他杀（`RoomCall::Doom`）：由别的域下的杀令。
pub(crate) const EXIT_DOOM: usize = 0xFFFF_FFFE;

/// 级联（父域退出 ⇒ 沿 heir 扑杀整棵子树）。
pub(crate) const EXIT_CASCADE: usize = 0xFFFF_FFFD;

// 子模块对外重导出：**外部路径一行不改**（`messenger::cull` 等照旧）。
pub(crate) use doom::{cull, descends, doom, take_doomed};
pub(crate) use handoff::Handoff;
pub(crate) use reap::{hook, quit};
pub(crate) use wait::holder::Ticket;
pub(crate) use wait::site::WakeKey;
pub(crate) use wait::{join, park, redeem, wait, wake, wipe, wipe_space};

// ── 操作：回收 ──

/// 终末释放：清空 messenger 持有的全部 `Arc<Task>`（sites / husks）与全部票根
/// ——Arc<Task> 归零 → Task::drop → MailHolds::drop → 链。由
/// [`scheduler::core::rip`] 在 halt 路径调用。
///
/// 逐表 `take` 出内容、**锁外 drop**：drop 链会取 Space 锁（L2），锁内 drop 即
/// L3→L2 嵌套。站点表分片版循环逐片取——不持跨片锁。
pub(crate) fn rip() {
    for shard in 0..SITE_SHARDS {
        let sites_out = core::mem::take(&mut *shard_at(shard).lock());
        drop(sites_out);
    }
    let husks_out = core::mem::take(&mut *HUSKS.lock());
    drop(husks_out);
    holders().lock().clear(); // 只存 Weak，无 drop 链
    doomed().lock().clear(); // 只存 id，无 Arc
}

/// 另两张簿记表的规模：票根（只存 `Weak`，无 drop 链）与躯壳队列。
///
/// 消费者是停机信标：挂住时它要报出"还剩多少簿记没清"，靠的就是这两个读数。
#[cfg(feature = "audit")]
pub(crate) fn probe_bookkeeping() -> (usize, usize) {
    let holders_n = holders().lock().len();
    let husks_n = HUSKS.lock().len();
    (holders_n, husks_n)
}

// ── 操作：扑杀（suspend / reap / cull / doom）──
//
// 血缘级联的「杀」侧，**两阶段**：先停摆（摘出全部调度/等待容器），再收尾
// （`reap`：钩子 → Reaped → 入躯壳队列）。`doom` 是 hook 里的触发面
// （读父 task 的 heir → 整棵子树两阶段扑杀）。
//
// 两阶段是**正确性要求**，不是优化：钩子会摘门闩，摘门闩会唤醒等待者；若受害者
// 尚未停摆，它可能被别的核偷走并运行，在「已注定要死」的状态下观察到一个已死的
// 资源。他核 Running 任务无法被本核同步拉走（会破坏「Reaped 不在 running 槽」
// 不变量），故走 `doomed` 待杀集合 + SSIP 单点，目标核 trap 自查自退——最终一致。
