// ── 适配层：trap ──
//
// 陷阱路径入口：run——取活/轮转统一入口。运行状态查询已并入核心（ident() 身份
// 槽，无锁、不设查询门面）。

use crate::machine;
use crate::runtime::diagnose::trace::{self, EventKind, SchedEvent};
use crate::work::room::tie;
use crate::work::unit::task::{Task, TaskState};

use super::core::{conductors, steal, wait};

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
            let pa = cur.ident.trap.pa.as_usize();
            i.running = Some(cur);
            return pa;
        }
        if i.starved.is_empty() {
            // 本 hart 唯一任务：预算耗尽但无处轮转，续跑
            let pa = cur.ident.trap.pa.as_usize();
            i.running = Some(cur);
            return pa;
        }
        let prev_tid = cur.ident.id;
        let next = s.rotate(&mut i, cur);
        let next_tid = next.ident.id;
        drop(i);
        trace::note(EventKind::Sched(SchedEvent::Switch { prev_tid, next_tid }));
        return s.mount(next);
    }
    drop(i);
    // 取活：Idle → Running 阻塞获取（自核队首 → 跨核 steal → 全退出检查 → WFI）
    loop {
        let me = machine::hart_id();
        if let Some(pa) = conductors()[me].pop().map(|t| conductors()[me].mount(t)) {
            return pa;
        }
        if tie::done() {
            tie::halt();
        }
        if let Some(task) = steal() {
            return conductors()[me].mount(task);
        }
        if let Some(task) = wait() {
            return conductors()[me].mount(task);
        }
    }
}