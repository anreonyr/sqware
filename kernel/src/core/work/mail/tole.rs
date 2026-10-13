use super::Liveness;
use crate::core::work::room::messenger::{self, WakeKey};
use crate::core::work::unit::join::{JoinTarget, JoinWait};
use crate::core::work::{
    pend,
    unit::{life::Life, task::Task},
};
use crate::machine::sync::{Level, SpinLock};
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;
use env::{AwaitReply, MailCondition, MailFail, PieToken, Source, TaskId};

const MAX: usize = 1024;

#[derive(Clone)]
pub(crate) enum Watch {
    Mail {
        pie: PieToken,
        condition: MailCondition,
    },
    Join {
        target: env::UnitTarget,
        selected: JoinTarget,
    },
    Inspect {
        task: TaskId,
        token: PieToken,
        holder: Weak<Task>,
        parent: Option<(Weak<Task>, PieToken)>,
        held: bool,
        access: env::Permission,
    },
}

impl Watch {
    pub(crate) fn source(&self) -> Source {
        match self {
            Watch::Mail { pie, condition } => Source::Mail {
                pie: *pie,
                condition: *condition,
            },
            Watch::Join { target, .. } => Source::Join { target: *target },
            Watch::Inspect { task, token, .. } => Source::Inspect {
                task: *task,
                token: *token,
            },
        }
    }
}

#[derive(Clone)]
pub(crate) struct Cell {
    pub actor: TaskId,
    pub keys: Arc<Vec<(WakeKey, Weak<Life>)>>,
    pub watch: Watch,
}
pub struct ToleMeta {
    state: SpinLock<Liveness>,
    pub(crate) id: usize,
    life: Arc<Life>,
    cells: SpinLock<Vec<Cell>>,
    owner: TaskId,
    cursor: AtomicUsize,
}
fn key(meta: &ToleMeta) -> WakeKey {
    WakeKey::Tole { id: meta.id }
}
impl ToleMeta {
    #[cfg(debug_assertions)]
    pub(crate) fn new(owner: TaskId) -> Arc<Self> {
        Self::try_new(owner).expect("tole allocation failed")
    }

    pub(crate) fn try_new(owner: TaskId) -> Result<Arc<Self>, crate::core::memory::manager::MapError> {
        Arc::try_new(Self {
            state: SpinLock::new_level(Level::L3, Liveness::Live),
            id: super::next_id(),
            life: Life::try_new()?,
            cells: SpinLock::new_level(Level::L3, Vec::new()),
            owner,
            cursor: AtomicUsize::new(0),
        })
        .map_err(|_| crate::core::memory::manager::MapError::OutOfMemory)
    }

    pub(crate) fn life(&self) -> Weak<Life> {
        Arc::downgrade(&self.life)
    }
    pub(crate) fn owner(&self) -> TaskId {
        self.owner
    }
    pub(crate) fn alive(&self) -> bool {
        *self.state.lock() == Liveness::Live
    }
    pub(crate) fn has_cells(&self) -> bool {
        !self.cells.lock().is_empty()
    }
    #[cfg(debug_assertions)]
    pub(crate) fn cells_len(&self) -> usize {
        self.cells.lock().len()
    }
    pub(crate) fn poll(&self, task: &Arc<Task>) -> Result<AwaitReply, MailFail> {
        if !self.alive() {
            return Err(MailFail::Dead);
        }
        let count = self.cells.lock().len();
        if count == 0 {
            return Ok(AwaitReply::Pending);
        }
        let start = self.cursor.load(Ordering::Relaxed) % count;
        let mut own = false;
        for offset in 0..count {
            let at = (start + offset) % count;
            let cell = { self.cells.lock().get(at).cloned() };
            let Some(cell) = cell else { continue };
            if cell.actor != task.ident.id {
                continue;
            }
            own = true;
            let status = cell.poll(task);
            if let Some(fail) = status {
                self.cursor.store((at + 1) % count, Ordering::Relaxed);
                return Ok(AwaitReply::Source {
                    source: cell.watch.source(),
                    fail,
                });
            }
        }
        if own {
            Ok(AwaitReply::Pending)
        } else {
            Err(MailFail::Denied)
        }
    }
}
impl Cell {
    fn poll(&self, actor: &Arc<Task>) -> Option<Option<MailFail>> {
        match &self.watch {
            Watch::Mail { pie, condition } => {
                let need = pend::Need::of(*condition);
                let snapshot = match pend::accede(actor, *pie, need) {
                    Ok(p) => p,
                    Err(e) => return Some(Some(super::mail_fail(e))),
                };
                if let Err(e) = pend::available(&snapshot) {
                    return Some(Some(e));
                }
                let ready = snapshot.mail().ready(*condition);
                ready.then_some(None)
            }
            Watch::Join { selected, .. } => {
                if let JoinTarget::Team(team) = selected {
                    if team.next(actor.ident.id).is_some() {
                        return Some(None);
                    }
                }
                let join = JoinWait::new(selected.clone(), actor.ident.id, false, env::Wait::POLL);
                match join.poll() {
                    Ok(env::JoinReply::Pending) => None,
                    Ok(_) => Some(None),
                    Err(_) => Some(Some(MailFail::Denied)),
                }
            }
            Watch::Inspect {
                token,
                holder,
                parent,
                held,
                access,
                ..
            } => {
                let Some(holder) = holder.upgrade() else {
                    return Some(Some(MailFail::Denied));
                };
                let Some(pie) = pend::locate(&holder, *token) else {
                    return Some(Some(MailFail::Denied));
                };
                if !pie.alive() {
                    return Some(Some(MailFail::Dead));
                }
                if !pie.permission().contains(*access) {
                    return Some(Some(MailFail::Denied));
                }
                if let Some((lord, parent_token)) = parent {
                    let Some(lord) = lord.upgrade() else {
                        return Some(Some(MailFail::Denied));
                    };
                    if lord.ident.id != actor.ident.id
                        || pend::locate(&lord, *parent_token).is_none()
                        || pie.sire() != Some(*parent_token)
                        || !pie.lord().ptr_eq(&Arc::downgrade(&lord))
                    {
                        return Some(Some(MailFail::Denied));
                    }
                    match pend::available(&pie) {
                        Ok(()) => None,
                        Err(e) => Some(Some(e)),
                    }
                } else {
                    match pend::available(&pie) {
                        Ok(()) => (*held).then_some(None),
                        Err(MailFail::HandedOver) if *held => None,
                        Err(e) => Some(Some(e)),
                    }
                }
            }
        }
    }
}

pub(crate) fn attach(meta: &ToleMeta, cell: Cell) -> Result<(), MailFail> {
    let _commit = crate::core::work::unit::commit();
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    let mut shared = [false; 4];
    {
        let mut cells = meta.cells.lock();
        if cells
            .iter()
            .any(|old| old.actor == cell.actor && old.watch.source() == cell.watch.source())
        {
            return Ok(());
        }
        if cells.len() >= MAX {
            return Err(MailFail::OoM);
        }
        cells.try_reserve(1).map_err(|_| MailFail::OoM)?;
        for (at, (key, _)) in cell.keys.iter().enumerate() {
            shared[at] = cells
                .iter()
                .any(|old| old.keys.iter().any(|(k, _)| k == key));
        }
    }
    for (at, (key, life)) in cell.keys.iter().enumerate() {
        if !shared[at] && messenger::forward(*key, life.clone(), meta.id, meta.life()).is_err() {
            for (index, (key, _)) in cell.keys.iter().take(at).enumerate() {
                if !shared[index] {
                    messenger::unforward(*key, meta.id);
                }
            }
            return Err(MailFail::OoM);
        }
    }
    meta.cells.lock().push(cell);
    messenger::knock(key(meta), &meta.life());
    Ok(())
}
pub(crate) fn detach(meta: &ToleMeta, actor: TaskId, source: Source) -> Result<(), MailFail> {
    let _commit = crate::core::work::unit::commit();
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    let cell = {
        let mut cells = meta.cells.lock();
        cells
            .iter()
            .position(|c| c.actor == actor && c.watch.source() == source)
            .map(|at| cells.remove(at))
    };
    if let Some(cell) = cell {
        for (key, _) in cell.keys.iter() {
            let shared = meta
                .cells
                .lock()
                .iter()
                .any(|c| c.keys.iter().any(|(k, _)| k == key));
            if !shared {
                messenger::unforward(*key, meta.id);
            }
        }
    }
    messenger::knock(key(meta), &meta.life());
    Ok(())
}
pub(crate) fn seal(meta: &ToleMeta) {
    let _commit = crate::core::work::unit::commit();
    clear(meta);
}
fn clear(meta: &ToleMeta) {
    *meta.state.lock() = Liveness::Dead;
    messenger::wipe(WakeKey::seal(env::PieKind::Tole, meta.id));
    let cells = core::mem::take(&mut *meta.cells.lock());
    for cell in cells {
        for (key, _) in cell.keys.iter() {
            messenger::unforward(*key, meta.id);
        }
    }
    messenger::wipe(key(meta));
}
impl Drop for ToleMeta {
    fn drop(&mut self) {
        clear(self);
    }
}

#[derive(Clone)]
pub(crate) struct AwaitWait {
    pub group: Arc<ToleMeta>,
    pub token: PieToken,
    pub actor: Weak<Task>,
    pub deadline: Option<u64>,
}
impl AwaitWait {
    pub fn new(group: Arc<ToleMeta>, token: PieToken, actor: &Arc<Task>, wait: env::Wait) -> Self {
        Self {
            group,
            token,
            actor: Arc::downgrade(actor),
            deadline: match wait {
                env::Wait::Forever => None,
                _ => Some(
                    crate::machine::clock::now()
                        .add(wait.into_duration())
                        .as_ticks(),
                ),
            },
        }
    }
    pub fn key(&self) -> WakeKey {
        key(&self.group)
    }
    pub fn life(&self) -> Weak<Life> {
        self.group.life()
    }
    pub fn watch(&self) -> Result<(), MailFail> {
        let actor = self.actor.upgrade().ok_or(MailFail::Denied)?;
        messenger::forward(
            WakeKey::Inspect {
                task: actor.ident.id,
                token: self.token.get(),
            },
            actor.life(),
            self.group.id,
            self.group.life(),
        )
        .map_err(|_| MailFail::OoM)
    }
    pub fn unwatch(&self) {
        if let Some(actor) = self.actor.upgrade() {
            messenger::unforward(
                WakeKey::Inspect {
                    task: actor.ident.id,
                    token: self.token.get(),
                },
                self.group.id,
            );
        }
    }
    pub fn remaining(&self) -> Duration {
        self.deadline.map_or(Duration::MAX, |at| {
            crate::machine::clock::ticks_to_duration(
                at.saturating_sub(crate::machine::clock::now().as_ticks()),
            )
        })
    }
    pub fn poll(&self) -> Result<AwaitReply, MailFail> {
        let actor = self.actor.upgrade().ok_or(MailFail::Denied)?;
        let pie = pend::accede(&actor, self.token, pend::Need::Fetch).map_err(super::mail_fail)?;
        pend::available(&pie)?;
        self.group.poll(&actor)
    }
    pub fn write(
        frame: &mut crate::machine::switcher::context::TrapContext,
        result: Result<AwaitReply, MailFail>,
    ) {
        use crate::machine::switcher::context::Gprs;
        let words = match result {
            Ok(reply) => reply.words(),
            Err(e) => [env::FailCode::code(e) as usize, 0, 0],
        };
        frame.gpr.set_x(Gprs::A0, words[0]);
        frame.gpr.set_x(Gprs::A1, words[1]);
        frame.gpr.set_x(Gprs::A2, words[2]);
    }
}

unsafe impl Send for ToleMeta {}
unsafe impl Sync for ToleMeta {}
unsafe impl Send for AwaitWait {}
unsafe impl Sync for AwaitWait {}
