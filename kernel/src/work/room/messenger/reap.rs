// 收割（reap）——「离核 → 收尾 → 埋掉」这条链与钩子注入面。
//
// 三个动词成因果串（都是 4 字母），入口是 `quit`：
//   `quit` 退场并交班 = 离核退出 → `reap` 收尾（钩子 → Reaped → 入躯壳队列）
//                     → `bury` 埋掉归还 → `scheduler::trap::run` 交下一帧
// 延迟的是**回收**（栈 / trap 帧 / 团队空间），不是收尾：不能在自己正在用的栈上
// 回收自己。故收尾在 `reap` 里就做完，`bury` 只管归还。
//
// `bury` 是 `quit` 的**内部一步**（私有）：排空必须发生在再次取活之前，把这条
// 不变量做进结构，就不必指望每个调用点记得按顺序写两行。

use alloc::collections::VecDeque;
use alloc::sync::Arc;

use crate::lock::{Level, OnceLock, SpinLock};
use crate::runtime::diagnose::trace::{self, EventKind, RoomEvent};
use crate::work::room::conductor;
use crate::work::room::scheduler::core::current;
use crate::work::unit::task::{Task, TaskState};

use super::{WakeKey, wipe};

/// 全局躯壳队列（Level::L3，与 Team.tasks 同级）：延迟回收——不能在
/// 自己正在用的栈上回收自己；bury 统一回收。
pub(super) static HUSKS: SpinLock<VecDeque<Arc<Task>>> =
    SpinLock::new_level(Level::L3, VecDeque::new());

/// 死亡唯一入口：**收尾**（退出钩子：通道级联 + 能力级联）→ 置 `Reaped` → 入躯壳队列。
///
/// 不变量：`TaskState::Reaped` ⇔ 退出钩子已跑完——本函数是通往 `Reaped` 的**唯一**
/// 路径，也是躯壳队列的**唯一**入队点。`Join` 的判据（入口当场读 `TaskState::Reaped`）
/// 因此精确：它为真即「收尾已完成」。
///
/// 前置：任务已停摆（`Doomed`）；或从自退路径来（此刻已离核、状态仍是 `Running`，
/// 本函数就地补一次停摆）。已 `Reaped` 的直接返回。
///
/// 锁纪律：无锁调用。钩子只逐任务取放 L3（`Task.pies` / 通道注册表），且 [`cull`]
/// 已把整棵子树的受害者停摆在前——故钩子内再扑杀子域，也不会唤醒「还能跑」的人。
pub(super) fn reap(mut task: Arc<Task>) {
    match task.state() {
        TaskState::Reaped => return,
        TaskState::Doomed => {}
        _ => Task::exclusive(&mut task).transform(TaskState::Doomed),
    }
    hooked(task.ident.id);
    Task::exclusive(&mut task).transform(TaskState::Reaped);
    HUSKS.lock().push_back(task); // L3 单独锁，1 → 3 顺序、不嵌套
}

/// quit：**退场并交班**——离核装槽 → [`reap`]（收尾 + 入壳）→ [`bury`]（排空躯壳）
/// → 交出下一个要恢复的帧。
///
/// 延迟回收的理由是**回收**而非收尾：不能在自己正在用的栈上回收自己，故栈/trap
/// 帧/团队空间留到 `bury`；收尾（钩子）在此刻就做完了。
///
/// 「排空躯壳」是本函数的一部分，**不是调用方的义务**：它必须发生在再次取活之前
/// ——最后退出的任务若带着栈/trap 帧/团队空间滞留到关机断言，就是帧泄漏。四个调用点
/// （envcall 的 `Exit`、fault isolation 的三个杀点）因此各自只写一行。
///
/// 落点由 `scheduler::trap::run` 决定（续跑 / 轮转 / 取活 / 停机）——它只会循环到
/// 有帧或停机，故恒有帧可交。
///
/// 注：`disown_and_install_next` 其实已经在装槽时给出了后继帧 PA，这里仍走 `run()`
/// 取活——两条路等价，差别只在 `run()` 会替后继再扣 1 个量子（8 → 7）。为与改前
/// 保持**逐字相同的调度行为**，本轮不动它（记一笔，待单独裁决）。
pub fn quit() -> usize {
    let cond = current();
    // 离核且无后继装槽 → 槽已 settled（disown_and_install_next 内 shed 或
    // 装下一）；团队 Arc 归零即回收——地址空间随释放。
    let (exited, _next_pa) = cond.disown_and_install_next();
    debug_assert!(
        matches!(exited.state(), TaskState::Running { .. }),
        "running 容器里不是 Running 任务"
    );
    trace::note(EventKind::Room(RoomEvent::Exit {
        tid: exited.ident.id,
    }));
    reap(exited);
    bury();
    crate::work::room::scheduler::trap::run()
}

/// 回收全部躯壳任务：簿记清理 + 栈 slot/trap 帧归还 + drop。安全：躯壳不在任何核
/// 运行（running/starved 均无引用）。锁纪律：只持 reaped 锁出队，放锁后再取
/// Team.tasks / Space.inner（顺序获取、不嵌套）。
///
/// **入队的任务已经收尾**（退出钩子见 [`reap`]），本函数只做回收——「等收尾」与
/// 「等回收」因此分开：前者是 `Join` 的语义，后者对调用方不可观测。
///
/// 唯一调用者是 [`quit`]（排空必须发生在再次取活之前，故是它的一部分）。回收计数
/// （`conductor::exit`）在归还完成之后才递增：否则最后任务退出时另一核见
/// `REAPED == PUSHED` 立即 halt，本核 bury 未及回收 → 关机断言误报帧泄漏。
fn bury() {
    loop {
        // 显式作用域取 z：if-let 的临时 guard 会存活到整个循环体（Rust 语义），
        // 导致 husks(L3) 锁跨 Team.tasks 等 L3 表——同层嵌套 lockdep 违规。
        // 块结束即释放 husks 锁。
        let z = {
            let mut husks = HUSKS.lock();
            let Some(z) = husks.pop_front() else {
                break;
            };
            z
        };
        trace::note(EventKind::Room(RoomEvent::Reap { tid: z.ident.id }));
        // 目标已收尾 → 叫醒它的全部 join 等待者（内核驱动，覆盖 fault 死亡）。
        // 站点当场删掉：`WakeKey::Task{id}` 的寿命是目标任务的存活单元，而本站点在
        // 键还在世的最后一次入口上——删掉它，站点表就不随任务回收增长（见 `wipe`）。
        wipe(WakeKey::Task { id: z.ident.id });
        // 簿记清理（Team.tasks 锁；纯 Vec 操作——不变量：锁内不调 space 方法）
        z.ident.team.prune_tasks(&z);
        // 锁外回收（Team.tasks 已放 → Space.inner=2 合法）：栈 slot + trap 帧
        // 一次 with_flush 经 `Space::release(Span)` 收回——段归还 + PTE 清理 +
        // 刷 TLB；帧随 map drop 归还 frame 池。Span 是 claim 时存进 TaskIdent 的
        // 区间身份（类型同一，不 re-find）。
        z.ident
            .team
            .space
            .release(z.ident.stack)
            .expect("release: span mismatch");
        z.ident
            .team
            .space
            .release(z.ident.frame)
            .expect("release: span mismatch");
        drop(z);
        // 回收完成（栈/帧/团队空间已归还）才计数：done() 成立 ⇔ 全部回收完毕，
        // halt 的关机断言无滞留可验。
        conductor::exit();
    }
}

// ── 退出钩子注册面 ──
//
// mail（dock / ring）在 `boot::init` 把自己的任务退出函数挂到这里。每条收尾的
// 任务按注册顺序跑一次——本域不命名任何子系统，故不知道挂上来的是谁。
type Hook = fn(usize);

static HOOKS: OnceLock<&'static [Hook]> = OnceLock::new();

/// 挂上退出钩子（一次性；由 `boot::init` 调用）。
pub(crate) fn hook(hooks: &'static [Hook]) {
    let _ = HOOKS.set(hooks);
}

/// 对 `tid` 跑一遍挂上的钩子（未挂 = 无事）。
fn hooked(tid: usize) {
    if let Some(hooks) = HOOKS.get() {
        for h in hooks.iter() {
            h(tid);
        }
    }
}
