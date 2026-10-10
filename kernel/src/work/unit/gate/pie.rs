use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::boxed::Box;
use alloc::sync::{Arc, Weak};

use env::{Mark, PieToken, TaskId};

use super::GateFail;

use crate::work::mail::nole::NoleMeta;
use crate::work::mail::{HoleMeta, PoleMeta, ToleMeta};
use crate::work::unit::task::Task;

pub(crate) trait Mail: Send + Sync + 'static {
    fn kind(&self) -> env::PieKind;
    fn owner(&self) -> TaskId;
    fn alive(&self) -> bool;
    fn seal(&self);
    fn grant(
        self: Arc<Self>,
        mark: Mark,
        permission: Permission,
        sire: PieToken,
    ) -> Result<AnyPie, env::PieFail>;

    fn hole(self: Arc<Self>) -> Option<Arc<HoleMeta>> {
        None
    }
    fn pole(self: Arc<Self>) -> Option<Arc<PoleMeta>> {
        None
    }
    fn nole(self: Arc<Self>) -> Option<Arc<NoleMeta>> {
        None
    }
    fn tole(self: Arc<Self>) -> Option<Arc<ToleMeta>> {
        None
    }

    fn permit(
        &self,
        _permission: Permission,
    ) -> Result<Option<Arc<super::super::space::Permit>>, env::PieFail> {
        Ok(None)
    }
}

impl Mail for HoleMeta {
    fn kind(&self) -> env::PieKind {
        env::PieKind::Hole
    }
    fn owner(&self) -> TaskId {
        HoleMeta::owner(self)
    }
    fn alive(&self) -> bool {
        HoleMeta::alive(self)
    }
    fn seal(&self) {
        crate::work::mail::hole::seal(self);
    }
    fn hole(self: Arc<Self>) -> Option<Arc<HoleMeta>> {
        Some(self)
    }
    fn grant(
        self: Arc<Self>,
        mark: Mark,
        permission: Permission,
        sire: PieToken,
    ) -> Result<AnyPie, env::PieFail> {
        boxed(try_new_pie::<Hole>(self, mark, permission, Some(sire))?)
    }
}

impl Mail for PoleMeta {
    fn kind(&self) -> env::PieKind {
        env::PieKind::Pole
    }
    fn owner(&self) -> TaskId {
        PoleMeta::owner(self)
    }
    fn alive(&self) -> bool {
        PoleMeta::alive(self)
    }
    fn seal(&self) {
        crate::work::mail::pole::seal(self);
    }
    fn pole(self: Arc<Self>) -> Option<Arc<PoleMeta>> {
        Some(self)
    }
    fn grant(
        self: Arc<Self>,
        mark: Mark,
        permission: Permission,
        sire: PieToken,
    ) -> Result<AnyPie, env::PieFail> {
        boxed(try_new_pie::<Pole>(self, mark, permission, Some(sire))?)
    }
    fn permit(
        &self,
        permission: Permission,
    ) -> Result<Option<Arc<super::super::space::Permit>>, env::PieFail> {
        self.backing()
            .try_permit(permission)
            .map(Some)
            .map_err(|_| env::PieFail::OoM)
    }
}

impl Mail for NoleMeta {
    fn kind(&self) -> env::PieKind {
        env::PieKind::Nole
    }
    fn owner(&self) -> TaskId {
        NoleMeta::owner(self)
    }
    fn alive(&self) -> bool {
        NoleMeta::alive(self)
    }
    fn seal(&self) {
        crate::work::mail::nole::seal(self);
    }
    fn nole(self: Arc<Self>) -> Option<Arc<NoleMeta>> {
        Some(self)
    }
    fn grant(
        self: Arc<Self>,
        mark: Mark,
        permission: Permission,
        sire: PieToken,
    ) -> Result<AnyPie, env::PieFail> {
        boxed(try_new_pie::<Nole>(self, mark, permission, Some(sire))?)
    }
}

impl Mail for ToleMeta {
    fn kind(&self) -> env::PieKind {
        env::PieKind::Tole
    }
    fn owner(&self) -> TaskId {
        ToleMeta::owner(self)
    }
    fn alive(&self) -> bool {
        ToleMeta::alive(self)
    }
    fn seal(&self) {
        crate::work::mail::tole::seal(self);
    }
    fn tole(self: Arc<Self>) -> Option<Arc<ToleMeta>> {
        Some(self)
    }
    fn grant(
        self: Arc<Self>,
        mark: Mark,
        permission: Permission,
        sire: PieToken,
    ) -> Result<AnyPie, env::PieFail> {
        boxed(try_new_pie::<Tole>(self, mark, permission, Some(sire))?)
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
    PieToken::mint(
        NEXT_ID
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("Pie token identity exhausted"),
    )
}

pub struct Pie<T: PieType> {
    pub(crate) permission: Permission,
    pub(crate) sire: Option<PieToken>,
    pub(crate) lord: Weak<Task>,
    pub(crate) heir: Option<Heir>,
    pub(crate) token: PieToken,
    pub(crate) mark: T::Mark,
    pub(crate) meta: Arc<T::Mail>,
    pub(crate) permit: Option<Arc<super::super::space::Permit>>,
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
            lord: self.lord.clone(),
            heir: self.heir,
            token: self.token,
            mark: self.mark,
            meta: self.meta.clone(),
            permit: self.permit.clone(),
        }
    }
}

impl<T: PieType> Pie<T> {
    pub(crate) fn permission(&self) -> Permission {
        self.permit
            .as_ref()
            .map_or(self.permission, |permit| permit.permission())
    }
}

pub(crate) fn form_ok(src: Permission, subset: Permission) -> bool {
    src.contains(Permission::ONLY) == subset.contains(Permission::ONLY)
}

/// One independently mutable reference in a task's table. Resource entities
/// remain shared; snapshots never allocate another boxed reference.
pub(crate) type AnyPie = Box<dyn PieOps>;

pub(crate) trait PieOps: Send {
    fn token(&self) -> PieToken;
    fn permission(&self) -> Permission;
    fn sire(&self) -> Option<PieToken>;
    fn lord(&self) -> &Weak<Task>;
    fn heir(&self) -> Option<&Heir>;
    #[cfg(debug_assertions)]
    fn mark(&self) -> Mark;
    fn meta(&self) -> &dyn Mail;
    fn permit(&self) -> Option<&Arc<super::super::space::Permit>>;
    fn snapshot(&self) -> PieSnapshot;
    fn parent(&mut self, token: PieToken, task: Weak<Task>);
    fn set_heir(&mut self, heir: Option<Heir>);
    fn set_permission(&mut self, subset: Permission);

    fn kind(&self) -> env::PieKind {
        self.meta().kind()
    }
    fn alive(&self) -> bool {
        self.meta().alive()
    }
    fn owner_task(&self) -> TaskId {
        self.meta().owner()
    }
    #[cfg(debug_assertions)]
    fn owner(&self) -> Option<TaskId> {
        self.alive().then(|| self.owner_task())
    }
    fn allows(&self, need: Need) -> bool {
        allows_permission(self.permission(), need)
    }
    fn invalidate(&self) {
        if let Some(permit) = self.permit() {
            permit.invalidate();
        }
    }
    fn narrow(&mut self, subset: Permission) -> Result<(), env::PieFail> {
        if !self.alive() {
            return Err(env::PieFail::Dead);
        }
        let permission = self.permission();
        if subset.is_empty()
            || !permission.contains(subset)
            || permission.contains(Permission::ONLY) && !subset.contains(Permission::ONLY)
        {
            return Err(env::PieFail::Denied);
        }
        self.set_permission(subset);
        Ok(())
    }
}

impl<T> PieOps for Pie<T>
where
    T: PieType<Mark = Mark>,
    Pie<T>: Send + 'static,
{
    fn token(&self) -> PieToken {
        self.token
    }
    fn permission(&self) -> Permission {
        Pie::permission(self)
    }
    fn sire(&self) -> Option<PieToken> {
        self.sire
    }
    fn lord(&self) -> &Weak<Task> {
        &self.lord
    }
    fn heir(&self) -> Option<&Heir> {
        self.heir.as_ref()
    }
    #[cfg(debug_assertions)]
    fn mark(&self) -> Mark {
        self.mark
    }
    fn meta(&self) -> &dyn Mail {
        self.meta.as_ref()
    }
    fn permit(&self) -> Option<&Arc<super::super::space::Permit>> {
        self.permit.as_ref()
    }
    fn snapshot(&self) -> PieSnapshot {
        PieSnapshot {
            token: self.token,
            permission: self.permission,
            mark: self.mark,
            sire: self.sire,
            lord: self.lord.clone(),
            heir: self.heir,
            meta: self.meta.clone(),
            permit: self.permit.clone(),
        }
    }
    fn parent(&mut self, token: PieToken, task: Weak<Task>) {
        self.sire = Some(token);
        self.lord = task;
    }
    fn set_heir(&mut self, heir: Option<Heir>) {
        self.heir = heir;
    }
    fn set_permission(&mut self, subset: Permission) {
        self.permission = subset;
        if let Some(permit) = &self.permit {
            permit.narrow(subset);
        }
    }
}

pub(crate) fn boxed<T>(pie: Pie<T>) -> Result<AnyPie, env::PieFail>
where
    T: PieType<Mark = Mark>,
    Pie<T>: Send + 'static,
{
    Ok(Box::try_new(pie).map_err(|_| env::PieFail::OoM)?)
}

/// Copies reference facts and shares the original resource/permit allocations.
/// Retaining this snapshot keeps memory valid, not authority: operations still
/// validate resource state, permits and the existing task relations.
#[derive(Clone)]
pub(crate) struct PieSnapshot {
    pub(crate) token: PieToken,
    permission: Permission,
    mark: Mark,
    pub(crate) sire: Option<PieToken>,
    lord: Weak<Task>,
    heir: Option<Heir>,
    meta: Arc<dyn Mail>,
    permit: Option<Arc<super::super::space::Permit>>,
}

impl PieSnapshot {
    pub(crate) fn token(&self) -> PieToken {
        self.token
    }
    pub(crate) fn permission(&self) -> Permission {
        self.permit
            .as_ref()
            .map_or(self.permission, |p| p.permission())
    }
    pub(crate) fn kind(&self) -> env::PieKind {
        self.meta.kind()
    }
    pub(crate) fn sire(&self) -> Option<PieToken> {
        self.sire
    }
    pub(crate) fn lord(&self) -> &Weak<Task> {
        &self.lord
    }
    pub(crate) fn heir(&self) -> Option<&Heir> {
        self.heir.as_ref()
    }
    pub(crate) fn mark(&self) -> Mark {
        self.mark
    }
    pub(crate) fn owner_task(&self) -> TaskId {
        self.meta.owner()
    }
    pub(crate) fn owner(&self) -> Option<TaskId> {
        self.alive().then(|| self.owner_task())
    }
    pub(crate) fn alive(&self) -> bool {
        self.meta.alive()
    }
    pub(crate) fn allows(&self, need: Need) -> bool {
        allows_permission(self.permission(), need)
    }
    pub(crate) fn covers(&self, subset: Permission) -> bool {
        self.permission().contains(subset) && !subset.is_empty()
    }
    pub(crate) fn hole(&self) -> Option<Arc<HoleMeta>> {
        self.meta.clone().hole()
    }
    pub(crate) fn pole(&self) -> Option<Arc<PoleMeta>> {
        self.meta.clone().pole()
    }
    pub(crate) fn nole(&self) -> Option<Arc<NoleMeta>> {
        self.meta.clone().nole()
    }
    pub(crate) fn tole(&self) -> Option<Arc<ToleMeta>> {
        self.meta.clone().tole()
    }
    pub(crate) fn seal(&self) {
        self.meta.seal();
    }
    pub(crate) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.meta, &other.meta)
    }
    pub(crate) fn grant(&self, mark: Mark, permission: Permission) -> Result<AnyPie, env::PieFail> {
        self.meta.clone().grant(mark, permission, self.token)
    }
}

fn allows_permission(permission: Permission, need: Need) -> bool {
    permission.contains(match need {
        Need::Fetch => Permission::FETCH,
        Need::Store => Permission::STORE,
        Need::Grant => Permission::VEST,
    })
}

pub(crate) fn new_pie<T: PieType>(
    meta: Arc<T::Mail>,
    mark: T::Mark,
    permission: Permission,
    sire: Option<PieToken>,
) -> Pie<T> {
    try_new_pie(meta, mark, permission, sire).expect("pie: allocation")
}

pub(crate) fn try_new_pie<T: PieType>(
    meta: Arc<T::Mail>,
    mark: T::Mark,
    permission: Permission,
    sire: Option<PieToken>,
) -> Result<Pie<T>, env::PieFail> {
    Ok(Pie {
        permit: meta.permit(permission)?,
        permission,
        sire,
        lord: Weak::new(),
        heir: None,
        token: alloc_id(),
        mark,
        meta,
    })
}

pub(crate) fn locate(task: &Task, token: PieToken) -> Option<PieSnapshot> {
    let pies = task.gate.pies.lock();
    pies.iter()
        .find(|p| p.token() == token)
        .map(|p| p.snapshot())
}

pub(crate) fn accede<E: GateFail>(
    task: &Arc<Task>,
    token: PieToken,
    need: Need,
) -> Result<PieSnapshot, E> {
    let pie = locate(task, token).ok_or_else(E::denied)?;
    if !pie.alive() {
        return Err(E::dead());
    }
    if !pie.allows(need) {
        return Err(E::denied());
    }
    Ok(pie)
}

pub(crate) fn allows<M: Mail>(task: &Task, resource: &Arc<M>, need: Need) -> bool {
    let _gate = task.gate.lock();
    task.gate.pies.lock().iter().any(|pie| {
        core::ptr::addr_eq(pie.meta() as *const dyn Mail, Arc::as_ptr(resource))
            && pie.alive()
            && pie.heir().is_none()
            && pie.allows(need)
    })
}
