use alloc::sync::Arc;

use env::{NOTE_MAX, TaskId};

use crate::lock::{Level, OnceLock, SpinLock};
use crate::runtime::diagnose::ledger;
use crate::runtime::diagnose::trace::{self, EventKind, RoomEvent};
use crate::work::room::conductor;
use crate::work::room::scheduler::core::current;
use crate::work::unit::space::Space;
use crate::work::unit::task::{Task, TaskExitCause, TaskState, TaskTag};

use super::{WakeKey, wipe, wipe_space};

pub(super) static HUSKS: SpinLock<Husks> = SpinLock::new_level(Level::L3, Husks::new());

#[derive(Default)]
pub(super) struct Husks {
    head: Option<Arc<Task>>,
    tail: Option<Arc<Task>>,
}

impl Husks {
    const fn new() -> Self {
        Self {
            head: None,
            tail: None,
        }
    }

    fn push(&mut self, task: Arc<Task>) {
        debug_assert!(
            matches!(task.tag(), TaskTag::Reaped),
            "躯壳容器只收 Reaped 任务，且入壳前不得挂在链上"
        );
        match self.tail.take() {
            None => self.head = Some(task.clone()),
            Some(mut last) => Task::set_reaped_next(&mut last, Some(task.clone())),
        }
        self.tail = Some(task);
    }

    fn pop(&mut self) -> Option<Arc<Task>> {
        let mut head = self.head.take()?;
        self.head = Task::take_reaped_next(&mut head);
        if self.head.is_none() {
            self.tail = None;
        }
        Some(head)
    }

    #[cfg(debug_assertions)]
    pub(super) fn len(&self) -> usize {
        let mut n = 0usize;
        let mut cur = self.head.clone();
        while let Some(node) = cur {
            n += 1;
            cur = Task::reaped_next(&node);
        }
        n
    }

    pub(super) fn take(&mut self) -> Option<Arc<Task>> {
        self.tail = None;
        self.head.take()
    }
}

pub(super) fn reap(mut task: Arc<Task>, cause: TaskExitCause, reason: usize) {
    let (cause, reason) = {
        let _commit = crate::work::unit::commit();
        let mut state = task.state.lock();
        match &*state {
            TaskState::Reaped { .. } => return,
            TaskState::Doomed { cause, reason, .. } => {
                let exit = (*cause, *reason);
                *state = TaskState::Doomed {
                    hart: None,
                    cause: exit.0,
                    reason: exit.1,
                };
                exit
            }
            _ => {
                *state = TaskState::Doomed {
                    hart: None,
                    cause,
                    reason,
                };
                (cause, reason)
            }
        }
    };
    hooked(&task);
    {
        let _commit = crate::work::unit::commit();
        task.transform(TaskState::Reaped { cause, reason });
    }
    super::signal(WakeKey::Task { id: task.ident.id });
    task.ident.team.life.notify();
    HUSKS.lock().push(task);
}

pub fn quit() -> usize {
    let cond = current();
    let requested = super::take_exit_reason();
    let (exited, _next_pa, reason) = {
        let _commit = crate::work::unit::commit();
        let (task, next) = cond.swap();
        let mut state = task.state.lock();
        let (cause, reason) = match &*state {
            TaskState::Doomed { cause, reason, .. } => (*cause, *reason),
            _ => (TaskExitCause::Reap, requested),
        };
        // Publish detachment in the same critical section as slot removal.
        *state = TaskState::Doomed {
            hart: None,
            cause,
            reason,
        };
        drop(state);
        (task, next, reason)
    };
    debug_assert!(
        matches!(
            exited.tag(),
            TaskTag::Running | TaskTag::Debarking | TaskTag::Doomed
        ),
        "running 容器里不是 Running 任务"
    );
    let (note_va, note_len) = super::take_exit_note();
    let tid = exited.ident.id;
    let mut buf = [0u8; NOTE_MAX];
    let text = note_out(
        &exited.ident.team.space,
        tid,
        reason,
        note_va,
        note_len,
        &mut buf,
    );
    trace::note(EventKind::Room(RoomEvent::Exit {
        tid: tid.get(),
        reason,
    }));
    // Existing receipt ownership assigns result policy to the manager. This
    // diagnostic snapshot requires no Observe call or mirrored Task flag.
    let owner = {
        let _commit = crate::work::unit::commit();
        exited
            .ident
            .team
            .life
            .member(tid)
            .and_then(|m| m.receipts.lock().first().map(|r| r.owner))
    };
    ledger::note(tid, reason, text, owner);
    reap(exited, TaskExitCause::Reap, reason);
    bury();
    crate::work::room::scheduler::trap::run()
}

fn note_out<'a>(
    space: &Space,
    tid: TaskId,
    reason: usize,
    va: usize,
    len: usize,
    buf: &'a mut [u8; NOTE_MAX],
) -> &'a str {
    if len == 0 {
        return "";
    }
    let n = len.min(NOTE_MAX);
    if !crate::work::mail::copy_in(space, &mut buf[..n], va) {
        crate::putln!(
            "exit tid={} reason={reason:#x} note=<unreadable {len} bytes at {va:#x}>",
            tid.get()
        );
        return "";
    }
    match core::str::from_utf8(&buf[..n]) {
        Ok(text) => {
            crate::putln!("exit tid={} reason={reason:#x} note: {text}", tid.get());
            text
        }
        Err(_) => {
            crate::putln!(
                "exit tid={} reason={reason:#x} note: <non-utf8 note>",
                tid.get()
            );
            "<non-utf8 note>"
        }
    }
}

fn bury() {
    loop {
        let z = {
            let mut husks = HUSKS.lock();
            let Some(z) = husks.pop() else {
                break;
            };
            z
        };
        trace::note(EventKind::Room(RoomEvent::Reap {
            tid: z.ident.id.get(),
        }));
        wipe(WakeKey::Task { id: z.ident.id });
        wipe(WakeKey::Pies { task: z.ident.id });
        if Arc::strong_count(&z.ident.team.space) == 1 {
            wipe_space(z.ident.team.space.asid());
        }
        z.ident.team.prune_tasks(&z);
        let _pruned = crate::work::room::scheduler::core::prune_dead();
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
        conductor::exit();
    }
}

/// 退场那一趟的钩子：**给的是这一具任务本身，不是它的号**。
type Hook = fn(&Arc<Task>);

static HOOKS: OnceLock<&'static [Hook]> = OnceLock::new();

pub(crate) fn hook(hooks: &'static [Hook]) {
    let _ = HOOKS.set(hooks);
}

fn hooked(task: &Arc<Task>) {
    if let Some(hooks) = HOOKS.get() {
        for h in hooks.iter() {
            h(task);
        }
    }
}
