use alloc::vec::Vec;
use env::{Mark, Permission, PieKind, PieToken};
use system_api::supply::{Fail, Key};

use super::{
    LeaseRequest, Provision, ProvisionError, Resource,
    lease::{DeliveryRoute, MAX_ROOTS, PrivateRoot, RootRequest},
    pole::PoleResource,
};

#[derive(Clone, Copy)]
pub struct CollectionItem {
    pub shared: PieToken,
    pub slot: Mark,
    pub kind: PieKind,
    pub permission: Permission,
}

/// One lease that installs a read-only collection Pole and direct target-local leaves.
pub struct CollectionResource<P, A> {
    key: Key,
    manifest: PoleResource<P>,
    items: Vec<CollectionItem>,
    admission: A,
}

pub struct CollectionSpec<P, A> {
    pub key: Key,
    pub manifest: PoleResource<P>,
    pub items: Vec<CollectionItem>,
    pub admission: A,
}

#[must_use = "a rejected collection still owns its manifest root"]
pub struct CollectionInitError<P> {
    failure: Fail,
    manifest: Option<PoleResource<P>>,
}

impl<P> CollectionInitError<P> {
    pub fn failure(&self) -> Fail {
        self.failure
    }
}

impl<P> CollectionInitError<P>
where
    P: Fn(&LeaseRequest) -> bool,
{
    pub fn retry_cleanup(&mut self) -> Result<(), Fail> {
        let Some(manifest) = self.manifest.as_mut() else {
            return Ok(());
        };
        manifest.shutdown()?;
        self.manifest = None;
        Ok(())
    }
}

impl<P, A> CollectionResource<P, A>
where
    P: Fn(&LeaseRequest) -> bool,
    A: Fn(&LeaseRequest) -> bool,
{
    pub fn new(spec: CollectionSpec<P, A>) -> Result<Self, CollectionInitError<P>> {
        let CollectionSpec {
            key,
            manifest,
            items,
            admission,
        } = spec;
        if key.name().is_empty()
            || items.len() + 1 > MAX_ROOTS
            || items.iter().enumerate().any(|(at, item)| {
                item.shared == PieToken::NONE
                    || item.slot == Mark::NONE
                    || item.permission.is_empty()
                    || items[..at].iter().any(|prior| prior.slot == item.slot)
            })
        {
            return Err(CollectionInitError {
                failure: Fail::Bad,
                manifest: Some(manifest),
            });
        }
        Ok(Self {
            key,
            manifest,
            items,
            admission,
        })
    }
}

impl<P, A> Resource for CollectionResource<P, A>
where
    P: Fn(&LeaseRequest) -> bool,
    A: Fn(&LeaseRequest) -> bool,
{
    fn key(&self) -> Key {
        self.key
    }

    fn authorize(&self, request: &LeaseRequest) -> bool {
        if !(self.admission)(request) || !self.manifest.authorize(request) {
            return false;
        }
        let actor = request.owner;
        self.items.iter().all(|item| {
            let Ok(info) = resource::raw::inspect(item.shared) else {
                return false;
            };
            let needed = item.permission | Permission::VEST;
            info.alive
                && info.kind == item.kind
                && info.permission.contains(needed)
                && info.permission.contains(Permission::ONLY)
                    == item.permission.contains(Permission::ONLY)
                && (info.owner == actor || info.vestor == actor)
        })
    }

    fn acquire(&mut self, request: &LeaseRequest) -> Result<Provision, ProvisionError> {
        if !self.authorize(request) {
            return Err(ProvisionError::new(Fail::Denied));
        }
        let mut provision = self.manifest.acquire(request)?;
        let mut roots = core::mem::take(&mut provision.roots);
        if roots.try_reserve(self.items.len()).is_err() {
            return Err(ProvisionError::with_roots(Fail::Full, roots));
        }
        for (at, item) in self.items.iter().enumerate() {
            let index = at + 1;
            let root = match PrivateRoot::derive(
                item.shared,
                RootRequest {
                    id: request.id,
                    origin: request.origin,
                    owner: request.owner,
                    permission: item.permission,
                },
                index,
            ) {
                Ok(root) => root,
                Err(failure) => return Err(ProvisionError::with_roots(failure, roots)),
            };
            roots.push(root);
        }
        let mut completed = Provision::new(
            roots,
            0,
            &provision.description[..provision.description_len as usize],
        )?;
        for (at, item) in self.items.iter().enumerate() {
            if let Err(failure) = completed.route(DeliveryRoute {
                root: at + 1,
                slot: item.slot,
                permission: item.permission,
            }) {
                return Err(ProvisionError::with_roots(failure, completed.roots));
            }
        }
        Ok(completed)
    }

    fn shutdown(&mut self) -> Result<(), Fail> {
        self.manifest.shutdown()
    }
}
