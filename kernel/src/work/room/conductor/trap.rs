// ── 适配层：trap ──
//
// 陷阱路径：取活/轮转统一入口 + 运行状态查询（正常路径与崩溃现场共用）。

use alloc::sync::Arc;

use crate::machine;
use crate::runtime::diagnose::trace::{self, EventKind, SchedEvent};
use crate::work::room::tie;
use crate::work::unit::{
    elftable::ElfTable,
    space::{Space, SpaceKind},
    task::{Task, TaskState},
    team::Team,
};

use super::core::{conductors, steal, wait, CONDUCTORS, NO_TASK_ID};

/// 统一入口：running 预算 > 1 → 续跑（只减计数不重排）；== 1 → 转 Starved
/// 轮转；无 running → 取活（自核队首 → 跨核 steal → WFI）。
///
/// 取活与抢占合一：有 running 走预算检查；空分支（running 已 take 或本为
/// 空闲）直接进入取活循环。
pub fn run() -> usize {
    // 0. 多核 panic：警报已拉响且本 hart 非报警源 → 就地卧倒（不返回）。
    //    覆盖空闲/WFI 核经 wait() 在**内核态**处理 IPI 唤醒的路径；常运行
    //    时恒 no-op。
    crate::runtime::diagnose::halt::hush();
    let me = machine::hart_id();
    let s = &conductors()[me];
    let mut i = s.inner.lock();
    if let Some(mut cur) = i.running.take() {
        let ticks_left = match cur.state() {
            TaskState::Running { ticks_left } => ticks_left,
            _ => unreachable!("running 容器里不是 Running 任务"),
        };
        if ticks_left > 1 {
            // 预算未耗尽：续跑——不切走、不进 starved
            Task::exclusive(&mut cur).dec_ticks_left();
            let pa = cur.trap.pa.as_usize();
            i.running = Some(cur);
            return pa;
        }
        if i.starved.is_empty() {
            // 本 hart 唯一任务：预算耗尽但无处轮转，续跑
            let pa = cur.trap.pa.as_usize();
            i.running = Some(cur);
            return pa;
        }
        let prev_tid = cur.id;
        let next = s.rotate(&mut i, cur);
        let next_tid = next.id;
        drop(i);
        trace::note(EventKind::Sched(SchedEvent::Switch { prev_tid, next_tid }));
        return s.replace(next);
    }
    drop(i);
    // 取活：Idle → Running 阻塞获取（自核队首 → 跨核 steal → 全退出检查 → WFI）
    loop {
        let me = machine::hart_id();
        if let Some(pa) = conductors()[me].pop().map(|t| conductors()[me].replace(t)) {
            return pa;
        }
        if tie::done() {
            tie::halt();
        }
        if let Some(task) = steal() {
            return conductors()[me].replace(task);
        }
        if let Some(task) = wait() {
            return conductors()[me].replace(task);
        }
    }
}

/// 是否有运行任务。
///
/// 与 `with_running_space` 的 expect 语义互补：正常运行路径必然有运行任务
/// （不减不 panic）；早期 boot/panic 现场可能没有——判 `false` 即静默丢弃
/// 用户窗口缓冲，绝不嵌套 panic。
pub fn has_running_task() -> bool {
    let me = machine::hart_id();
    conductors()[me].inner.lock().running.is_some()
}

/// 在当前运行任务的空间上执行闭包（锁内借出，引用不逃逸锁）。
pub fn with_running_space<R>(f: impl FnOnce(&Space) -> R) -> R {
    let me = machine::hart_id();
    let i = conductors()[me].inner.lock();
    let task = i.running.as_ref().expect("no running task");
    f(&task.team.space)
}

/// 当前运行任务所属团队 Arc（锁内取、放锁返回）。
///
/// 供 envcall Spawn 使用：放锁后再建任务，不得跨锁持有；持有的 Arc 保团队
/// 存活。无运行任务则 panic。
pub fn running_team() -> Arc<Team> {
    let me = machine::hart_id();
    conductors()[me]
        .inner
        .lock()
        .running
        .as_ref()
        .expect("no running task")
        .team
        .clone()
}

/// 当前运行任务所属团队（panic/诊断现场安全）：读**本核**调度器的 running_team
/// 镜像——不碰调度锁（崩溃时它常被持会失败），镜像锁仅本核装槽瞬间持、无争用。
/// 返回 clone 的 Arc<Team>（放锁后安全使用）。per-hart 各一：精确到核（本核
/// 最近上台的 team；idle 时保留旧值——符号化无碍）。未装槽（boot 早期 / 无
/// 任务）→ None。
pub fn running_team_try() -> Option<Arc<Team>> {
    let all = CONDUCTORS.get()?;
    let me = machine::hart_id();
    let guard = all[me].running_team.try_lock()?;
    guard.as_ref().map(|t| t.clone())
}

/// 当前运行任务 id（诊断用；无任务返回 NO_TASK_ID）。
pub fn running_task_id() -> usize {
    let me = machine::hart_id();
    conductors()[me]
        .inner
        .lock()
        .running
        .as_ref()
        .map(|t| t.id)
        .unwrap_or(NO_TASK_ID)
}

/// 当前运行任务 id + 名称（panic 诊断用；非阻塞，失败返回 None）。
///
/// 与 `running_task_id` 不同，本函数专供 panic 现场：panic 路径故意绕过所有锁，
/// 故这里走两个防御——调度器尚未初始化（极早期 boot panic）直接返回 None；
/// 调度锁被 panic 现场持有（持锁处 panic）则 `try_lock` 拿不到立即放弃，避免
/// 递归死锁。拿到的 id/name 均为可拷贝数据，锁随作用域即放。
pub fn running_task_info() -> Option<(usize, &'static str)> {
    // 调度器未初始化（boot 早期 panic）时无可查。
    let all = CONDUCTORS.get()?;
    let me = machine::hart_id();
    let guard = all[me].inner.try_lock()?;
    guard.running.as_ref().map(|t| (t.id, t.name))
}

/// 当前运行任务 trap 帧物理地址 + 其团队符号表 + 空间 kind（崩溃现场用；非阻塞，
/// 失败返回 None）。
///
/// 与 `running_task_info` 同纪律：调度锁被 panic 现场持有则 `try_lock` 放弃。
/// 返回 (task id, 帧 PA, 团队 elftable, 空间 kind)——PA 经恒等映射可读；
/// elftable 为符号化提供与帧同源的符号表（不依赖 running_team 镜像）；kind 为
/// 抢占持久化与诊断的模式判定源（见 trap::persist_kernel_preempt）。
pub fn running_task_frame() -> Option<(usize, usize, Option<Arc<ElfTable>>, SpaceKind)> {
    let all = CONDUCTORS.get()?;
    let me = machine::hart_id();
    let guard = all[me].inner.try_lock()?;
    guard
        .running
        .as_ref()
        .map(|t| (t.id, t.trap.pa.as_usize(), t.team.elftable.clone(), t.team.space.kind()))
}