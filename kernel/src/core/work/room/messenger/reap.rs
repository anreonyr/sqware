use alloc::sync::Arc;

use env::{NOTE_MAX, TaskId};

use crate::core::work::record::ledger;
use crate::core::work::record::trace::{self, EventKind, RoomEvent};
use crate::core::work::room::scheduler::core::current;
use crate::core::work::room::{Chain, conductor};
use crate::core::work::unit::space::Space;
use crate::core::work::unit::task::{Task, TaskExitCause, TaskState};
use crate::machine::sync::{Level, OnceLock, SpinLock};

use super::{WakeKey, wipe, wipe_space};

pub(super) static HUSKS: SpinLock<Husks> = SpinLock::new_level(Level::L3, Husks::new());

pub(super) struct Husks {
    chain: Chain,
}

impl Husks {
    const fn new() -> Self {
        Self {
            chain: Chain::new(),
        }
    }

    fn push(&mut self, task: Arc<Task>) {
        debug_assert!(
            task.reaped(),
            "躯壳容器只收 Reaped 任务，且入壳前不得挂在链上"
        );
        self.chain.push(task);
    }

    fn pop(&mut self) -> Option<Arc<Task>> {
        self.chain.pop()
    }

    #[cfg(debug_assertions)]
    pub(super) fn len(&self) -> usize {
        self.chain.len()
    }

    pub(super) fn take(&mut self) -> Option<Arc<Task>> {
        self.chain.take()
    }
}

pub(super) fn reap(task: Arc<Task>, cause: TaskExitCause, reason: usize) {
    let (cause, reason) = {
        let _commit = crate::core::work::unit::commit();
        let mut state = task.state.lock();
        if state.exit().is_some() {
            return;
        }
        let (cause, reason) = state.dying().unwrap_or((cause, reason));
        state.doom(None, cause, reason);
        (cause, reason)
    };
    hooked(&task);
    {
        let _commit = crate::core::work::unit::commit();
        task.transform_checked(TaskState::reaped(cause, reason));
    }
    super::signal(WakeKey::Task { id: task.ident.id });
    task.ident.team.life.notify();
    HUSKS.lock().push(task);
}

pub fn quit() -> usize {
    let cond = current();
    let requested = super::take_exit_reason();
    let (exited, _next_pa, reason) = {
        let _commit = crate::core::work::unit::commit();
        let (task, next) = cond.swap();
        let mut state = task.state.lock();
        let (cause, reason) = state.dying().unwrap_or((TaskExitCause::Reap, requested));
        state.doom(None, cause, reason);
        drop(state);
        (task, next, reason)
    };
    debug_assert!(
        exited.on_hart().is_some() || exited.doomed(),
        "running 容器里不是在核上的任务"
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
    let owner = {
        let _commit = crate::core::work::unit::commit();
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
    crate::core::work::room::scheduler::trap::run()
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
    if !space.memory.copy_in(&mut buf[..n], va) {
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
        if z.ident.retire().is_err() {
            HUSKS.lock().push(z);
            break;
        }
        trace::note(EventKind::Room(RoomEvent::Reap {
            tid: z.ident.id.get(),
        }));
        wipe(WakeKey::Task { id: z.ident.id });
        wipe(WakeKey::Pies { task: z.ident.id });
        if Arc::strong_count(&z.ident.team.space) == 1 {
            wipe_space(z.ident.team.space.memory.asid());
        }
        z.ident.team.prune_tasks();
        let _pruned = crate::core::work::room::scheduler::core::prune_dead();
        drop(z);
        conductor::exit();
    }
}

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
