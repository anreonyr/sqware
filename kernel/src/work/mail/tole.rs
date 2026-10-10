use crate::lock::{Level, SpinLock};
use crate::work::room::messenger::{self, WakeKey};
use crate::work::unit::join::{JoinTarget, JoinWait};
use crate::work::unit::{gate, life::Life, task::Task};
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;
use env::{AwaitReply, MailCondition, MailFail, PieToken, Source, TaskId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ToleId(pub usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToleState {
    Live,
    Dead,
}
const MAX: usize = 1024;

#[derive(Clone)]
pub(crate) enum Member {
    Mail,
    Join(JoinTarget),
    Inspect {
        holder: Weak<Task>,
        parent: Option<(Weak<Task>, PieToken)>,
        held: bool,
        access: env::Permission,
    },
}
#[derive(Clone)]
pub(crate) struct Cell {
    pub source: Source,
    pub actor: TaskId,
    pub member: Member,
    pub keys: Arc<Vec<(WakeKey, Weak<Life>)>>,
}
pub struct ToleMeta {
    state: SpinLock<ToleState>,
    id: ToleId,
    life: Arc<Life>,
    cells: SpinLock<Vec<Cell>>,
    owner: TaskId,
    cursor: AtomicUsize,
}
pub(crate) fn key(meta: &ToleMeta) -> WakeKey {
    WakeKey::Tole { id: meta.id.0 }
}
impl ToleMeta {
    pub(crate) fn life(&self) -> Weak<Life> {
        Arc::downgrade(&self.life)
    }
    pub(crate) fn id(&self) -> ToleId {
        self.id
    }
    pub(crate) fn owner(&self) -> TaskId {
        self.owner
    }
    pub(crate) fn alive(&self) -> bool {
        *self.state.lock() == ToleState::Live
    }
    // Every registration is tied to the registering task. Transferring a populated
    // group cannot transfer source authority; Accord refuses this case.
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
            // Clone one registration, not the whole table. No allocation in scan.
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
                    source: cell.source,
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
        match &self.member {
            Member::Mail => {
                let Source::Mail { pie, condition } = self.source else {
                    unreachable!()
                };
                let need = if matches!(condition, MailCondition::Pull | MailCondition::Signal(_)) {
                    gate::Need::Fetch
                } else {
                    gate::Need::Store
                };
                let snapshot = match gate::accede::<MailFail>(actor, pie, need) {
                    Ok(p) => p,
                    Err(e) => return Some(Some(e)),
                };
                if let Err(e) = crate::runtime::switcher::envcall::pie::observe(&snapshot) {
                    return Some(Some(e));
                }
                let ready = if let Some(meta) = snapshot.hole() {
                    meta.ready(condition)
                } else if let Some(meta) = snapshot.nole() {
                    meta.ready()
                } else if let (Some(meta), MailCondition::Signal(bit)) =
                    (snapshot.pole(), condition)
                {
                    meta.ready(bit)
                } else {
                    return Some(Some(MailFail::Denied));
                };
                ready.then_some(None)
            }
            Member::Join(target) => {
                let join = JoinWait::new(target.clone(), actor.ident.id, false, env::Wait::POLL);
                match join.poll() {
                    Ok(env::JoinReply::Pending) => None,
                    Ok(_) => Some(None),
                    Err(_) => Some(Some(MailFail::Denied)),
                }
            }
            Member::Inspect {
                holder,
                parent,
                held,
                access,
            } => {
                let Source::Inspect { token, .. } = self.source else {
                    unreachable!()
                };
                let Some(holder) = holder.upgrade() else {
                    return Some(Some(MailFail::Denied));
                };
                let Some(pie) = gate::locate(&holder, token) else {
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
                        || gate::locate(&lord, *parent_token).is_none()
                        || pie.sire() != Some(*parent_token)
                        || !pie.lord().ptr_eq(&Arc::downgrade(&lord))
                    {
                        return Some(Some(MailFail::Denied));
                    }
                    // Existing, usable direct child remains owned by the endpoint.
                    match crate::runtime::switcher::envcall::pie::observe(&pie) {
                        Ok(()) => None,
                        Err(e) => Some(Some(e)),
                    }
                } else {
                    match crate::runtime::switcher::envcall::pie::observe(&pie) {
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
    let _commit = crate::work::unit::commit();
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    let mut shared = [false; 4];
    {
        let mut cells = meta.cells.lock();
        if cells
            .iter()
            .any(|old| old.actor == cell.actor && old.source == cell.source)
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
    // Unit serializes publication; the L3 registration lock must not be held
    // while acquiring an L3 messenger site or waking another task.
    for (at, (key, life)) in cell.keys.iter().enumerate() {
        if !shared[at] && messenger::forward(*key, life.clone(), meta.id.0, meta.life()).is_err() {
            for (index, (key, _)) in cell.keys.iter().take(at).enumerate() {
                if !shared[index] {
                    messenger::unforward(*key, meta.id.0);
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
    let _commit = crate::work::unit::commit();
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    let cell = {
        let mut cells = meta.cells.lock();
        cells
            .iter()
            .position(|c| c.actor == actor && c.source == source)
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
                messenger::unforward(*key, meta.id.0);
            }
        }
    }
    messenger::knock(key(meta), &meta.life());
    Ok(())
}
pub(crate) fn seal(meta: &ToleMeta) {
    let _commit = crate::work::unit::commit();
    clear(meta);
}
fn clear(meta: &ToleMeta) {
    *meta.state.lock() = ToleState::Dead;
    messenger::wipe(WakeKey::Seal {
        kind: env::PieKind::Tole as u8,
        id: meta.id.0,
    });
    let cells = core::mem::take(&mut *meta.cells.lock());
    for cell in cells {
        for (key, _) in cell.keys.iter() {
            messenger::unforward(*key, meta.id.0);
        }
    }
    messenger::wipe(key(meta));
}
// The last Arc cannot race an Attach; dropping may occur beneath a gate lock.
impl Drop for ToleMeta {
    fn drop(&mut self) {
        clear(self);
    }
}
pub(crate) fn try_meta(owner: TaskId) -> Result<Arc<ToleMeta>, crate::memory::manager::MapError> {
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    Arc::try_new(ToleMeta {
        state: SpinLock::new_level(Level::L3, ToleState::Live),
        id: ToleId(NEXT.fetch_add(1, Ordering::Relaxed)),
        life: Life::try_new()?,
        cells: SpinLock::new_level(Level::L3, Vec::new()),
        owner,
        cursor: AtomicUsize::new(0),
    })
    .map_err(|_| crate::memory::manager::MapError::OutOfMemory)
}
#[cfg(debug_assertions)]
pub(crate) fn meta(owner: TaskId) -> Arc<ToleMeta> {
    try_meta(owner).expect("tole allocation failed")
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
                    crate::runtime::chrono::clock::now()
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
            self.group.id.0,
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
                self.group.id.0,
            );
        }
    }
    pub fn remaining(&self) -> Duration {
        self.deadline.map_or(Duration::MAX, |at| {
            crate::runtime::chrono::clock::ticks_to_duration(
                at.saturating_sub(crate::runtime::chrono::clock::now().as_ticks()),
            )
        })
    }
    pub fn poll(&self) -> Result<AwaitReply, MailFail> {
        let actor = self.actor.upgrade().ok_or(MailFail::Denied)?;
        let pie = gate::accede::<MailFail>(&actor, self.token, gate::Need::Fetch)?;
        crate::runtime::switcher::envcall::pie::observe(&pie)?;
        self.group.poll(&actor)
    }
    pub fn write(
        frame: &mut crate::runtime::switcher::context::TrapContext,
        result: Result<AwaitReply, MailFail>,
    ) {
        use crate::runtime::switcher::context::Gprs;
        let words = match result {
            Ok(reply) => reply.words(),
            Err(e) => [env::FailCode::code(e) as usize, 0, 0],
        };
        frame.gpr.set_x(Gprs::A0, words[0]);
        frame.gpr.set_x(Gprs::A1, words[1]);
        frame.gpr.set_x(Gprs::A2, words[2]);
    }
}

// SAFETY: kernel-only token values are identifiers; every observation validates
// authority under Unit and the original gate tables. Locks protect registrations.
unsafe impl Send for ToleMeta {}
unsafe impl Sync for ToleMeta {}
// SAFETY: the token is used only to revalidate the original weak task's gate;
// it does not authorize a different task or expose a transferable user handle.
unsafe impl Send for AwaitWait {}
unsafe impl Sync for AwaitWait {}
