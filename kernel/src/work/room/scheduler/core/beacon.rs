//! 停机信标 — 挂住时唯一的发声口。
//!
//! # 为什么必须单列一个模块
//!
//! 「关机偶发挂住」的症状是：`exit` 之后打印若干 `[audit] space asid N retired …`
//! 就再没有 `task: all tasks exited, system halted`，四个 hart 全停在空闲 `wfi`。
//! **挂住时没有任何 hart 在跑** —— 没有栈帧可读、没有 panic 现场可 dump，事后
//! 只能对着"少了哪一行"猜。本模块把那个"猜"变成一行日志：
//!
//! ```text
//! [stop] hart 1 空闲等待：PUSHED=13 REAPED=11（差 2）husks=0 在世任务 id=[7, 9]
//! ```
//!
//! 三种读数对应三种卡点，互不混淆：
//!
//! * `PUSHED - REAPED > 0` 且**在世 id 非空** ⇒ 还有任务活着（它没退，别人都在等它）；
//!   id 直接点名，接着查它为什么不停。
//! * `PUSHED - REAPED > 0` 而**在世 id 为空** ⇒ 任务都已析构，卡在**收尾队列**里
//!   （`husks > 0` 即 `bury` 没跑完；它是"躯壳入队但没人排空"的直接读数）。
//! * `PUSHED == REAPED` ⇒ 收尾做完了，卡在**停机屏障**（有核没到 `halt`），
//!   这一种由 `conductor::halt` 的屏障信标报（`[stop] halt 屏障等待：已达 x/4 核`）。
//!
//! # 代价与节制
//!
//! 只在**收尾期**观察 —— 判据是**根服务任务已死**（[`arm`] 注入，见 `boot`：
//! 根任务一退，`root: session over, shutting down` 就跟着来了），加上
//! `REAPED > 0 && REAPED < PUSHED`（已有任务回收、还没全退）。**这两条一起才叫
//! 收尾期**：会话中途坐在提示符前时，根任务还活着（尽管那时也可能有任务已回收、
//! 有任务还活着）—— 只按计数判会在那里误报（实测：`PUSHED=10 REAPED=1`、
//! 八个任务在世，正是 `badslot`/`stray` 那几步之间的空档）。且**全局只打一次**
//! （`BEACON_FIRED`）。判据是**时间**不是轮数：正常收尾（十几个任务依次退场）实测
//! `exit` → `system halted` 约 0.3~0.6 s，而卡住的收尾是**永远**；取 2 s 的窗口，
//! 两头都离得远（实测：按轮数计阈值会在正常收尾里误报 —— 那次的读数是
//! `PUSHED=10 REAPED=1`，正是收尾刚开头）。
//!
//! 产品档也带着它 —— 挂住不是 audit 档特有的现象，而这一行是唯一能指路的证据。

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use crate::work::unit::task::Task;

/// 停滞窗口：收尾**毫无进展**持续这么久就报。取 2 s —— 正常收尾全流程 0.3~0.6 s，
/// 而门的单步超时是 15 s，两侧都有 5 倍以上余量。
const STALL_MS: u64 = 2_000;

/// 上一次观察到**进展**的时刻（刻度）。0 = 尚未开始观察。
static LAST_PROGRESS: AtomicU64 = AtomicU64::new(0);
/// 上次观察到的 `(PUSHED, REAPED)`（打包成 u64 比较用；进展即刷新时刻）。
static LAST_COUNTS: AtomicU64 = AtomicU64::new(0);
/// 只打一次。
static BEACON_FIRED: AtomicBool = AtomicBool::new(false);

/// 根服务任务 **id**（boot 装出后注入）：**它一走，会话就结束了**。信标据此把
/// "收尾期"与"会话中途的空档"分开 —— 后者同样满足 `REAPED < PUSHED`（有任务已回收、
/// 有任务还活着），只按计数判会在那里误报。
///
/// **存 id 而不是 `Weak<Task>`**：静态里放一枚 `Weak` 就是**永久的弱引用**，而
/// `ArcInner<Task>`（152 B）的最后一门正是弱引用 —— 它会把这个任务的外壳一直扣到
/// 关机，于是停机普查每次都报任务外壳未归零。这不是推论：本轮真用 `Weak` 实现过
/// 一次，四轮里四轮复现，改成 id + 名册查名后归零。
/// 名册（[`super::table::muster`]）是**设计上的任务索引**，且 `rip` 会清空它 —— 用它
/// 查名不会给任何对象续命。
static ROOT_ID: AtomicUsize = AtomicUsize::new(0);

/// 注入根任务（`boot::spawn_root` 成功之后一次）。
pub(crate) fn arm(root: &Arc<Task>) {
    ROOT_ID.store(root.ident.id, Ordering::Relaxed);
}

/// **会话是否已结束**（收尾期的判据）。
///
/// 判据是**根任务的 `Reaped` 状态**，不是"根任务已析构"：根任务退出时会触发
/// `doom` 级联（它的整棵血缘子树在**它的 `reap` 里**被一个个收尾），而根任务自己的
/// 外壳要等它被 `bury` 才放掉 —— 于是"已析构"这个条件在正常收尾里**几乎不成立**
/// （实测：把判据写成 `strong_count == 0`，`STALL_MS=1` 也一次都不发声）。而
/// `Reaped` 在 `reap` 开头就置位、此后**恒真** ⇒ 它给出的窗口正好覆盖整段收尾。
///
/// 未注入 ⇒ 不发声；已析构（`upgrade` 失败）⇒ 会话当然早就结束了。
pub(super) fn shutting_down() -> bool {
    let id = ROOT_ID.load(Ordering::Relaxed);
    if id == 0 {
        return false;
    }
    match super::table::muster(id) {
        // 名册里已经没有它（`rip` 清空了名册）⇒ 早已过了收尾。
        None => true,
        Some(w) => match w.upgrade() {
            // 外壳还在、载荷没了 ⇒ 收尾已走到它后面。
            None => true,
            Some(t) => matches!(
                t.tag(),
                crate::work::unit::task::TaskTag::Reaped | crate::work::unit::task::TaskTag::Doomed
            ),
        },
    }
}

/// 停滞判定：`(pushed, reaped)` 与前一次不同 ⇒ 有进展，刷新时刻并返回 false。
fn stalled(pushed: usize, reaped: usize) -> bool {
    let now = crate::runtime::chrono::clock::now().as_ticks();
    let packed = ((pushed as u64) << 32) | (reaped as u64 & 0xffff_ffff);
    if LAST_COUNTS.swap(packed, Ordering::Relaxed) != packed
        || LAST_PROGRESS.load(Ordering::Relaxed) == 0
    {
        LAST_PROGRESS.store(now, Ordering::Relaxed);
        return false;
    }
    let t0 = LAST_PROGRESS.load(Ordering::Relaxed);
    let past = now.wrapping_sub(t0);
    past
        >= crate::runtime::chrono::clock::duration_to_ticks(core::time::Duration::from_millis(
            STALL_MS,
        ))
}

/// 空闲核每次决定睡下前调一次（见 [`super::fetch`] 的 WFI 循环）。
pub(super) fn idle(hart: usize) {
    if BEACON_FIRED.load(Ordering::Relaxed) {
        return;
    }
    let (pushed, reaped) = crate::work::room::conductor::counts();
    if !shutting_down() {
        return;
    }
    // 收尾期：已有任务回收、但还没全退。此外（启动期、会话期、满载期）一律不观察。
    if reaped == 0 || reaped >= pushed {
        LAST_PROGRESS.store(0, Ordering::Relaxed);
        return;
    }
    if !stalled(pushed, reaped) {
        return;
    }
    BEACON_FIRED.store(true, Ordering::Relaxed);
    #[cfg(feature = "audit")]
    {
        let (holders, husks) = crate::work::room::messenger::probe_bookkeeping();
        let (more, ids) = super::table::roster_live_ids();
        let live = {
            // 定长数组里前 n 个非零就算"在世"（id 自 1 起，0 是无效哨兵）。
            let mut n = 0usize;
            while n < ids.len() && ids[n] != 0 {
                n += 1;
            }
            n
        };
        crate::putln!(
            "[stop] hart {hart} 空闲等待：PUSHED={pushed} REAPED={reaped}（差 {}）husks={husks} \
             holders={holders} 在世任务 id={:?}{}",
            pushed.saturating_sub(reaped),
            &ids[..live],
            if more > 0 { " …" } else { "" }
        );
    }
    #[cfg(not(feature = "audit"))]
    {
        crate::putln!(
            "[stop] hart {hart} 空闲等待：PUSHED={pushed} REAPED={reaped}（差 {}）",
            pushed.saturating_sub(reaped)
        );
    }
}
