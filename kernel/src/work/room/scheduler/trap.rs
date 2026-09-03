// ── 适配层：trap ──
//
// 陷阱路径入口：run——取活/轮转统一入口。运行状态查询已并入核心（ident() 身份
// 槽，无锁、不设查询门面）。

use crate::runtime::diagnose::trace::{self, EventKind, RoomEvent};
use crate::work::room::conductor;
use crate::work::unit::task::{Task, TaskState};

use super::core::{current, steal, wait};

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
    let s = current();
    let mut i = s.inner.lock();
    if let Some(mut cur) = i.running.take() {
        let ticks_left = match cur.state() {
            TaskState::Running { ticks_left } => ticks_left,
            _ => unreachable!("running 容器里不是 Running 任务"),
        };
        // 续跑两分支合并（语义等价：先判后 dec——先判是否续跑，再在分支内递减；
        // 若先 dec 再判，pre=2 且他队非空会提前轮转一格）。两情形均不切走、
        // 不进 starved；预算恒 ≥ 1 不落盘（唯一任务分支不减预算）。
        if ticks_left > 1 || i.starved.is_empty() {
            if ticks_left > 1 {
                Task::exclusive(&mut cur).dec_ticks_left();
            }
            let pa = cur.ident.frame.pa.expect("frame span has pa").as_usize();
            i.running = Some(cur);
            return pa;
        }
        let prev_tid = cur.ident.id;
        let next = s.rotate(&mut i, cur);
        let next_tid = next.ident.id;
        drop(i);
        // Switch 事件落在身份槽更新（mount）**之后**：窗口内崩溃不再把已下台
        // 的 prev 报成当前任务（轮转窗口 issue）。
        let pa = s.mount(next);
        trace::note(EventKind::Room(RoomEvent::Switch { prev_tid, next_tid }));
        return pa;
    }
    drop(i);
    // 取活：Idle → Running 阻塞获取。顺序不可重排：done 检查须在 steal **之前**
    // （全退出后不得再取活——链式写法会把该检查挤进 wait() 内部，halt 语义后移）；
    // wait() 内部自带 done 复审 + 睡眠位协议（未到期/假醒自洽）。
    loop {
        if let Some(task) = s.pull() {
            return s.mount(task);
        }
        if conductor::done() {
            conductor::halt();
        }
        if let Some(task) = steal() {
            return s.mount(task);
        }
        if let Some(task) = wait() {
            return s.mount(task);
        }
    }
}
