use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::sync::Arc;

use env::{Mark, PieToken, TaskId};

use super::GateFail;

use crate::work::mail::nole::NoleMeta;
use crate::work::mail::{HoleMeta, PoleMeta, ToleMeta};
use crate::work::unit::task::Task;

pub(crate) trait Mail: Send + Sync + 'static {
    fn alive(&self) -> bool;
}

impl Mail for HoleMeta {
    fn alive(&self) -> bool {
        HoleMeta::alive(self)
    }
}

impl Mail for PoleMeta {
    fn alive(&self) -> bool {
        PoleMeta::alive(self)
    }
}

impl Mail for NoleMeta {
    fn alive(&self) -> bool {
        NoleMeta::alive(self)
    }
}

impl Mail for ToleMeta {
    fn alive(&self) -> bool {
        ToleMeta::alive(self)
    }
}

pub(crate) trait PieType {
    type Mail: Mail;
    type Mark;
}

pub(crate) struct Hole;
pub(crate) struct Pole;
pub(crate) struct Nole;
pub(crate) struct Tole;

impl PieType for Hole {
    type Mail = HoleMeta;
    type Mark = Mark;
}

impl PieType for Pole {
    type Mail = PoleMeta;
    type Mark = Mark;
}

impl PieType for Nole {
    type Mail = NoleMeta;
    type Mark = Mark;
}

impl PieType for Tole {
    type Mail = ToleMeta;
    type Mark = Mark;
}

pub use env::Permission;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Heir {
    pub(crate) task: TaskId,
    pub(crate) token: PieToken,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    Fetch,
    Store,
    Grant,
}

fn alloc_id() -> PieToken {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(1);
    PieToken::mint(NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

pub struct Pie<T: PieType> {
    pub(crate) permission: Permission,
    pub(crate) sire: Option<PieToken>,
    pub(crate) heir: Option<Heir>,
    pub(crate) token: PieToken,
    pub(crate) mark: T::Mark,
    pub(crate) meta: Arc<T::Mail>,
}

unsafe impl<T: PieType> Send for Pie<T> where T::Mark: Send {}

impl<T: PieType> Clone for Pie<T>
where
    T::Mark: Copy,
{
    fn clone(&self) -> Self {
        Self {
            permission: self.permission,
            sire: self.sire,
            heir: self.heir,
            token: self.token,
            mark: self.mark,
            meta: self.meta.clone(),
        }
    }
}

impl<T: PieType> Pie<T> {
    pub(crate) fn meta(&self) -> &Arc<T::Mail> {
        &self.meta
    }

    pub fn allows(&self, need: Need) -> bool {
        match need {
            Need::Fetch => self.permission.contains(Permission::FETCH),
            Need::Store => self.permission.contains(Permission::STORE),
            Need::Grant => self.permission.contains(Permission::VEST),
        }
    }

    pub fn covers(&self, subset: Permission) -> bool {
        self.permission.contains(subset) && !subset.is_empty()
    }
}

pub(crate) fn form_ok(src: Permission, subset: Permission) -> bool {
    src.contains(Permission::ONLY) == subset.contains(Permission::ONLY)
}

#[derive(Clone)]
pub enum AnyPie {
    Hole(Pie<Hole>),
    Pole(Pie<Pole>),
    Nole(Pie<Nole>),
    Tole(Pie<Tole>),
}

impl AnyPie {
    pub fn permission(&self) -> Permission {
        match self {
            AnyPie::Hole(p) => p.permission,
            AnyPie::Pole(p) => p.permission,
            AnyPie::Nole(p) => p.permission,
            AnyPie::Tole(p) => p.permission,
        }
    }

    pub fn sire(&self) -> Option<PieToken> {
        match self {
            AnyPie::Hole(p) => p.sire,
            AnyPie::Pole(p) => p.sire,
            AnyPie::Nole(p) => p.sire,
            AnyPie::Tole(p) => p.sire,
        }
    }

    pub fn heir(&self) -> Option<&Heir> {
        match self {
            AnyPie::Hole(p) => p.heir.as_ref(),
            AnyPie::Pole(p) => p.heir.as_ref(),
            AnyPie::Nole(p) => p.heir.as_ref(),
            AnyPie::Tole(p) => p.heir.as_ref(),
        }
    }

    pub fn owner(&self) -> Option<TaskId> {
        match self {
            AnyPie::Hole(p) => p.meta.alive().then(|| p.meta.owner()),
            AnyPie::Pole(p) => p.meta.alive().then(|| p.meta.owner()),
            AnyPie::Nole(p) => p.meta.alive().then(|| p.meta.owner()),
            AnyPie::Tole(p) => p.meta.alive().then(|| p.meta.owner()),
        }
    }

    pub fn owner_task(&self) -> TaskId {
        match self {
            AnyPie::Hole(p) => p.meta.owner(),
            AnyPie::Pole(p) => p.meta.owner(),
            AnyPie::Nole(p) => p.meta.owner(),
            AnyPie::Tole(p) => p.meta.owner(),
        }
    }

    pub fn token(&self) -> PieToken {
        match self {
            AnyPie::Hole(p) => p.token,
            AnyPie::Pole(p) => p.token,
            AnyPie::Nole(p) => p.token,
            AnyPie::Tole(p) => p.token,
        }
    }

    pub fn mark(&self) -> Mark {
        match self {
            AnyPie::Hole(p) => p.mark,
            AnyPie::Pole(p) => p.mark,
            AnyPie::Nole(p) => p.mark,
            AnyPie::Tole(p) => p.mark,
        }
    }

    pub fn alive(&self) -> bool {
        match self {
            AnyPie::Hole(p) => p.meta.alive(),
            AnyPie::Pole(p) => p.meta.alive(),
            AnyPie::Nole(p) => p.meta.alive(),
            AnyPie::Tole(p) => p.meta.alive(),
        }
    }

    pub fn allows(&self, need: Need) -> bool {
        match self {
            AnyPie::Hole(p) => p.allows(need),
            AnyPie::Pole(p) => p.allows(need),
            AnyPie::Nole(p) => p.allows(need),
            AnyPie::Tole(p) => p.allows(need),
        }
    }

    pub fn covers(&self, subset: Permission) -> bool {
        match self {
            AnyPie::Hole(p) => p.covers(subset),
            AnyPie::Pole(p) => p.covers(subset),
            AnyPie::Nole(p) => p.covers(subset),
            AnyPie::Tole(p) => p.covers(subset),
        }
    }
}

pub(crate) fn new_pie<T: PieType>(
    meta: Arc<T::Mail>,
    mark: T::Mark,
    permission: Permission,
    sire: Option<PieToken>,
) -> Pie<T> {
    Pie {
        permission,
        sire,
        heir: None,
        token: alloc_id(),
        mark,
        meta,
    }
}

pub(crate) fn locate(task: &Arc<Task>, token: PieToken) -> Option<AnyPie> {
    let pies = task.pies.lock();
    pies.iter().find(|p| p.token() == token).cloned()
}

pub(crate) fn accede<E: GateFail>(
    task: &Arc<Task>,
    token: PieToken,
    need: Need,
) -> Result<AnyPie, E> {
    let pie = locate(task, token).ok_or_else(E::denied)?;
    if !pie.alive() {
        return Err(E::dead());
    }
    if !pie.allows(need) {
        return Err(E::denied());
    }
    Ok(pie)
}