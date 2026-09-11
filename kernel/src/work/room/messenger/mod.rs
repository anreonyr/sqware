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
#[cfg(feature = "audit")]
use crate::work::unit::life::Life;
use doom::doomed;
use reap::HUSKS;
use wait::holder::{holders, void};
use wait::site::{SITE_SHARDS, prune, shard_at};
// `WakeKind` 只被 audit 档的观测面用（`SiteStats` 分列）——非 audit 构建下不引，
// 免得留下一条「导入了但没人读」的飞线。
#[cfg(feature = "audit")]
use wait::site::WakeKind;

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

// ── 观测面：站点表只读计数（audit 档） ──
//
// `prune`（空站点出队即删）每次收队都在跑，却**零观测量**：站点表规模从哪里都
// 读不到，于是「删没删」只能靠读代码断言。本面把规模变成可观测量——只读、不
// 改任何表（不加 envcall，ABI 不变）。
//
// **必须在 [`rip`] 之前读**：`rip` 的职责就是清空这两张表，之后读到的恒为 0，
// 那样的断言没有牙（`prune` 全坏也照样是 0）。消费者 = `boot::register_runtime_hooks`
// 的关机钩子（序列里排在 `rip` 之前的那一条）。

/// 站点表的一帧只读快照：合计规模 + 三形态分列 + 等待者总数。
///
/// 三形态（判据见 `site::prune`）：活 / 墓碑 / 孤儿。`tomb` 自 A2 落地后**恒为 0**
/// ——`wipe` 不再留墓碑，「此键已死」由 [`Life`] 承担（见 `work::unit::life`）。
/// 保留这个字段是**反向验证要的哨兵**：判据 1 就是「它必须掉到 0」，一个计数若被
/// 删掉就再也验不了「没有墓碑」（本轮实测 34 → 0；把判据与 `wipe` 一并改回原样 ⇒
/// 原样回到 34）。
///
/// 后两者必须分列的理由是**只有孤儿能指证 `prune`**：总数会把「活站点」与「残留」
/// 混在一起——本轮反向验证实测：把 `prune` 整个关掉，走完门那九步的总数只从 34 变
/// 36，而**孤儿从 0 变 2**；断言不分开看就几乎没有牙。
#[cfg(feature = "audit")]
pub(crate) struct SiteStats {
    /// 全部 16 片合计的站点数（键数）= 活 + 墓碑 + 孤儿。
    pub(crate) sites: usize,
    /// 还挂着等待者的站点数。
    pub(crate) live: usize,
    /// 墓碑站点数：队列空、但有信标。**不作为判据**（见下 `dead`）：键还活着时它
    /// 有语义——「`wake` 在无人在等时置的遗留信号」，下一个等待者会立刻消费它。
    /// 轮④ 挂上 `cascade` 后它稳定是 1，而那是**合法**状态。
    pub(crate) tomb: usize,
    /// **孤儿**站点数：队列空 **且** 无信标——`prune` 该删而没删的残留。
    pub(crate) orphan: usize,
    /// **死键站点数**：键的存活单元已死。A2「站点寿命＝资源寿命」的**精确**形式，
    /// 必须为 0。它与上面三形态**正交**（能入队 ⇒ 键活着，故 `live` 里不会有死键；
    /// 死键只能落在 `tomb`/`orphan` 里）。资源退役时 `wipe` 当场删站点，故一个死键
    /// 站点存在 ⇔ 某条退役路径漏了 `wipe`——这是 `tomb` 那种混合计数给不出的牙。
    pub(crate) dead: usize,
    /// 全部站点队列里的等待者总数（挂起任务数）。
    pub(crate) waiters: usize,
    /// 按 [`WakeKind::ALL`] 下标分列的站点数（四类合计 == `sites`）。
    /// 只用定长数组（关机路径上不为一行诊断再分配）。
    pub(crate) each: [usize; WakeKind::ALL.len()],
}

#[cfg(feature = "audit")]
impl SiteStats {
    /// 分列串：`space N hole N task N alarm N`。四类的名字与顺序**只有一处**出处
    /// （[`WakeKind::ALL`] / [`WakeKind::name`]），本方法不含第二份清单。
    pub(crate) fn kinds(&self) -> impl core::fmt::Display + '_ {
        struct Kinds<'a>(&'a [usize]);
        impl core::fmt::Display for Kinds<'_> {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                for (i, k) in WakeKind::ALL.iter().enumerate() {
                    if i > 0 {
                        f.write_str(" ")?;
                    }
                    write!(f, "{} {}", k.name(), self.0[i])?;
                }
                Ok(())
            }
        }
        Kinds(&self.each)
    }
}

/// 站点表快照：**逐片取、逐片放**（绝不持跨片锁，同 [`rip`] 的锁纪律）。
///
/// dropped 纪律：每片 `HashMap` 的值含 `Arc<Task>`，**锁外 drop**——持 L3 锁
/// drop `Arc<Task>` 会顺 drop 链取 Space 锁（L2），即 3→2 嵌套。
#[cfg(feature = "audit")]
pub(crate) fn probe() -> SiteStats {
    let mut st = SiteStats {
        sites: 0,
        live: 0,
        tomb: 0,
        orphan: 0,
        dead: 0,
        waiters: 0,
        each: [0; WakeKind::ALL.len()],
    };
    for shard in 0..SITE_SHARDS {
        // 作用域即临界区：`SpinLock::lock` 返的是**守卫**（不是引用），故守卫必须在
        // 块内成型、块末即放——出块后本片就没有 `Arc<Task>` 被持锁 drop 的风险。
        {
            let guard = shard_at(shard).lock();
            for (key, site) in guard.iter() {
                st.sites += 1;
                st.each[key.kind() as usize] += 1;
                if !site.waiters.is_empty() {
                    st.live += 1;
                } else if site.pend {
                    st.tomb += 1;
                } else {
                    st.orphan += 1;
                }
                if Life::dead(&site.life) {
                    st.dead += 1;
                }
                st.waiters += site.waiters.len();
            }
        }
    }
    st
}

/// 另两张簿记表的规模：票根（只存 `Weak`，无 drop 链）与躯壳队列。
///
/// 一并量出去的理由与站点表同：它们也只由关机钩子清，`rip` 之后就再也读不到。
/// `husks` 里若**还有东西**，说明 `bury` 没跑完 —— 而全部任务回收是停机的前置。
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
