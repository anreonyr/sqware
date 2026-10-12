//! Control owns identity installation; bootstrap is direct IPC, never Operator discovery.
//!
//! Static units are explicitly authorized with separate root-derived principals. Runtime
//! children inherit the kernel sender's current snapshot before embark. An unavailable
//! authority is not a reason to release a child without an identity.

use env::unit;
use env::{Access, Mark, Permission, PieKind, PieToken, TaskId, Wait};
use system_api::identity::Grant;
use system_api::identity::PrincipalId;
use system_api::identity::Subject;
use system_api::identity::{Install, InstallId, InstallSpec, InstallState, InstallStatus};
use system_client::identity::Installer;

use crate::support::timing::{BOOT_MS, RETRY_MS};
use core::time::Duration;
use ipc::session::establish;
use resource::raw::reserve;

const _: () = assert!(Grant::ALL.len() <= system_api::operator::frame::PANE_CAP);
pub(crate) const RESOLVE_CARRIER_MARK: Mark = Mark::of("identity/resolve-carrier");

#[derive(Default)]
pub(crate) struct Roster {
    installer: Option<Installer>,
    control: Option<PrincipalId>,
    next_install: u64,
    installations: alloc::vec::Vec<Installation>,
    deliveries: alloc::vec::Vec<CapabilityDelivery>,
}

#[derive(Clone, Copy)]
struct Installation {
    id: InstallId,
    spec: InstallSpec,
}

#[derive(Clone, Copy)]
struct CapabilityDelivery {
    key: DeliveryKey,
    state: DeliveryState,
}

#[derive(Clone, Copy)]
struct DeliveryKey {
    install: InstallId,
    task: TaskId,
    request: FaceKey,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct FaceKey {
    grant: Grant,
    access: Access,
}

#[derive(Clone, Copy)]
struct DeliveryState {
    root: PieToken,
    seed: PieToken,
    carrier: PieToken,
    carrier_owner: Option<TaskId>,
    authority: Option<RootAuthority>,
}

#[derive(Clone, Copy)]
struct RootAuthority {
    mark: Mark,
    provenance: Provenance,
    permission: Permission,
}

#[derive(Clone, Copy)]
struct Provenance {
    owner: TaskId,
    kind: PieKind,
}

#[derive(Clone, Copy)]
struct FaceRequest {
    install: InstallId,
    task: TaskId,
    grant: Grant,
    access: Access,
}
pub(crate) struct ForwardRequest {
    pub id: InstallId,
    pub task: TaskId,
    pub owner: TaskId,
}
pub(crate) struct IdentityAssociation {
    pub object: system_api::identity::IdentityObject,
    pub name: system_api::identity::IdentityName,
    pub entry: system_api::identity::OperatorEntryRef,
}

impl Roster {
    pub(crate) fn retire(&mut self) -> Result<(), &'static str> {
        while let Some(id) = self.installations.first().map(|row| row.id) {
            self.retire_install(id)?;
        }
        self.installer = None;
        self.control = None;
        Ok(())
    }

    pub(crate) fn control(&self) -> Option<PrincipalId> {
        self.control
    }

    pub(crate) fn authority(&self) -> Option<TaskId> {
        self.installer.as_ref().map(Installer::authority)
    }

    pub(crate) fn associate(&self, association: IdentityAssociation) -> Result<(), &'static str> {
        self.installer
            .as_ref()
            .ok_or("identity not installed")?
            .associate(
                association.object,
                association.name,
                association.entry,
                Wait::AtMost(BOOT_MS),
            )
            .map_err(|_| "identity association")
    }

    /// Give an external unit its own root-derived principal before embark.
    pub(crate) fn authorize(&mut self, task: TaskId) -> Result<InstallId, &'static str> {
        if self
            .installations
            .iter()
            .any(|entry| entry.spec.task == task)
        {
            let status = self.ensure_installed(task)?;
            self.require_installed(status)?;
            self.inject(task)?;
            return self
                .installations
                .iter()
                .find(|entry| entry.spec.task == task)
                .map(|entry| entry.id)
                .ok_or("identity installation missing");
        }
        let installer = self.installer.as_ref().ok_or("identity not installed")?;
        let root = PrincipalId::root(installer.authority());
        let (id, status) = self.install_status(task, Install::Derive { parent: root })?;
        self.require_installed(status)?;
        self.inject(task)?;
        Ok(id)
    }

    pub(crate) fn install(
        &mut self,
        task: TaskId,
        identity: Install,
    ) -> Result<InstallId, &'static str> {
        let (id, status) = self.install_status(task, identity)?;
        self.require_installed(status)?;
        self.inject(task)?;
        Ok(id)
    }

    /// Installs identity state without sending a Resolve seed to the child.
    /// Assembly must forward the retained Resolve root through the child's owner.
    pub(crate) fn install_owned(
        &mut self,
        task: TaskId,
        identity: Install,
    ) -> Result<InstallId, &'static str> {
        let (id, status) = self.install_status(task, identity)?;
        self.require_installed(status)?;
        Ok(id)
    }

    fn install_status(
        &mut self,
        task: TaskId,
        identity: Install,
    ) -> Result<(InstallId, InstallStatus), &'static str> {
        if let Some(record) = self
            .installations
            .iter()
            .find(|record| record.spec.task == task)
        {
            if record.spec.install != identity {
                return Err("identity installation conflict");
            }
            let id = record.id;
            return self.ensure_installed(task).map(|status| (id, status));
        }
        let id = InstallId {
            value: self
                .next_install
                .checked_add(1)
                .ok_or("identity installation ids exhausted")?,
        };
        self.installations
            .try_reserve(1)
            .map_err(|_| "identity installation capacity")?;
        self.next_install = id.value;
        let spec = InstallSpec {
            task,
            install: identity,
        };
        self.installations.push(Installation { id, spec });
        let status = self.retry_install(id, spec)?;
        Ok((id, status))
    }

    fn retry_install(
        &self,
        id: InstallId,
        spec: InstallSpec,
    ) -> Result<InstallStatus, &'static str> {
        let installer = self.installer.as_ref().ok_or("identity not installed")?;
        let mut status = InstallStatus {
            state: InstallState::Unseen,
            binding: None,
            failure: None,
        };
        for _ in 0..3 {
            match installer.install(id, spec, Wait::AtMost(BOOT_MS)) {
                Ok(reply) => return Ok(reply),
                Err(_) => match installer.inspect(id, Wait::AtMost(BOOT_MS)) {
                    Ok(reply) if reply.state != InstallState::Unseen => return Ok(reply),
                    Ok(reply) => status = reply,
                    Err(_) => {}
                },
            }
        }
        Ok(status)
    }

    pub(crate) fn inherit(
        &mut self,
        task: TaskId,
        parent: TaskId,
    ) -> Result<InstallId, &'static str> {
        self.install(task, Install::Inherit { parent })
    }

    fn inject(&mut self, task: TaskId) -> Result<(), &'static str> {
        let install = self
            .install_id(task)
            .ok_or("identity installation missing")?;
        self.deliver_face(FaceRequest {
            install,
            task,
            grant: Grant::Resolve,
            access: Access::FETCH_STORE,
        })
    }

    pub(crate) fn grant_face(&mut self, task: TaskId, grant: Grant) -> Result<(), &'static str> {
        let install = self
            .install_id(task)
            .ok_or("identity installation missing")?;
        self.deliver_face(FaceRequest {
            install,
            task,
            grant,
            access: Access::STORE,
        })
    }

    /// Give an installed owner a VEST carrier for forwarding that installation's Resolve face.
    /// The returned seed remains under the installation's private root and is revoked with it.
    pub(crate) fn forward_resolve(
        &mut self,
        request: ForwardRequest,
    ) -> Result<PieToken, &'static str> {
        let ForwardRequest { id, task, owner } = request;
        let installation = self
            .installations
            .iter()
            .find(|record| record.id == id)
            .ok_or("identity installation missing")?;
        if installation.spec.task != task || task.get() == 0 || owner.get() == 0 {
            return Err("identity resolve carrier task mismatch");
        }
        let status = self.ensure_installed(task)?;
        self.require_installed(status)?;
        let at = self.ensure_root(FaceRequest {
            install: id,
            task,
            grant: Grant::Resolve,
            access: Access::FETCH_STORE,
        })?;
        let state = self.deliveries[at].state;
        let authority = state.authority.ok_or("identity resolve root state")?;
        let root = state.root;
        if root == PieToken::NONE {
            return Err("identity resolve root missing");
        }
        let info = resource::raw::inspect(root).map_err(|_| "inspect identity resolve root")?;
        if !info.alive
            || info.owner != authority.provenance.owner
            || info.vestor != env::unit::self_id()
            || info.mark != authority.mark
            || info.kind != authority.provenance.kind
            || info.permission != authority.permission
            || !info.permission.contains(Permission::VEST)
        {
            return Err("identity resolve root changed");
        }
        let mark = RESOLVE_CARRIER_MARK;
        if state.carrier != PieToken::NONE {
            if state.carrier_owner != Some(owner) {
                return Err("identity resolve carrier owner changed");
            }
            // Accord's successful return is the receipt. The seed belongs to the owner's local
            // token namespace, so only that owner may inspect it.
            return Ok(state.carrier);
        }
        let permission = carrier_permission(info.permission);
        let seed = env::pie::accord(root, owner, permission, mark)
            .map_err(|_| "identity resolve carrier grant")?;
        self.deliveries[at].state.carrier = seed;
        self.deliveries[at].state.carrier_owner = Some(owner);
        Ok(seed)
    }

    fn deliver_face(&mut self, request: FaceRequest) -> Result<(), &'static str> {
        let at = self.ensure_root(request)?;
        let FaceRequest {
            task,
            grant,
            access,
            ..
        } = request;
        if self.deliveries[at].state.seed != PieToken::NONE {
            return Ok(());
        }
        let state = self.deliveries[at].state;
        let authority = state.authority.ok_or("identity grant root state")?;
        let child = access.bits() | (authority.permission & Permission::ONLY);
        let seed = env::pie::accord(state.root, task, child, grant.mark())
            .map_err(|_| "identity face grant")?;
        self.deliveries[at].state.seed = seed;
        Ok(())
    }

    fn ensure_root(&mut self, request: FaceRequest) -> Result<usize, &'static str> {
        let FaceRequest {
            install,
            task,
            grant,
            access,
        } = request;
        let at = if let Some(at) = self.deliveries.iter().position(|delivery| {
            delivery.key.install == install && delivery.key.request.grant == grant
        }) {
            at
        } else {
            self.deliveries
                .try_reserve(1)
                .map_err(|_| "identity delivery capacity")?;
            self.deliveries.push(CapabilityDelivery {
                key: DeliveryKey {
                    install,
                    task,
                    request: FaceKey { grant, access },
                },
                state: DeliveryState {
                    root: PieToken::NONE,
                    seed: PieToken::NONE,
                    carrier: PieToken::NONE,
                    carrier_owner: None,
                    authority: None,
                },
            });
            self.deliveries.len() - 1
        };
        if self.deliveries[at].key.request.access != access {
            return Err("identity delivery conflict");
        }
        if self.deliveries[at].key.task != task {
            return Err("identity delivery target changed");
        }
        let authority = current_authority(self).ok_or("identity authority stale")?;
        if self.deliveries[at].state.root == PieToken::NONE {
            let source = face_of(authority, grant)?;
            let info = resource::raw::inspect(source).map_err(|_| "identity grant source")?;
            let actor = env::unit::self_id();
            let mut permission = access.bits() | Permission::VEST;
            if info.permission.contains(Permission::ONLY) {
                permission |= Permission::ONLY;
            }
            if !info.alive
                || info.owner != authority
                || info.mark != grant.mark()
                || !info.permission.contains(permission)
            {
                return Err("identity grant source");
            }
            let mark = Mark::new(
                install.value.rotate_left(19)
                    ^ (grant.index() as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
                    ^ 0x4944_4752_414e_5401,
            );
            let root = env::pie::accord(source, actor, permission, mark)
                .map_err(|_| "identity grant root")?;
            self.deliveries[at].state.root = root;
            self.deliveries[at].state.authority = Some(RootAuthority {
                mark,
                provenance: Provenance {
                    owner: info.owner,
                    kind: info.kind,
                },
                permission,
            });
        }
        let state = self.deliveries[at].state;
        let authority = state.authority.ok_or("identity grant root state")?;
        let root = state.root;
        let info = resource::raw::inspect(root).map_err(|_| "identity grant root")?;
        if !info.alive
            || info.owner != authority.provenance.owner
            || info.vestor != env::unit::self_id()
            || info.mark != authority.mark
            || info.kind != authority.provenance.kind
            || info.permission != authority.permission
        {
            return Err("identity grant root changed");
        }
        Ok(at)
    }

    fn revoke_deliveries(&mut self, install: InstallId) -> Result<(), &'static str> {
        let mut at = 0;
        while at < self.deliveries.len() {
            if self.deliveries[at].key.install != install {
                at += 1;
                continue;
            }
            let delivery = &mut self.deliveries[at];
            let root = delivery.state.root;
            if root != PieToken::NONE {
                let info =
                    resource::raw::inspect(root).map_err(|_| "inspect identity grant root")?;
                if info.alive {
                    let authority = delivery
                        .state
                        .authority
                        .ok_or("identity grant root state")?;
                    if info.owner != authority.provenance.owner
                        || info.vestor != env::unit::self_id()
                        || info.mark != authority.mark
                        || info.kind != authority.provenance.kind
                        || info.permission != authority.permission
                    {
                        return Err("identity grant root changed");
                    }
                    resource::raw::release(root).map_err(|_| "revoke identity grant root")?;
                }
            }
            self.deliveries.remove(at);
        }
        Ok(())
    }

    pub(crate) fn unbind(&mut self, task: TaskId) -> Result<(), &'static str> {
        if self.installer.is_none() {
            return Ok(());
        }
        let Some(at) = self
            .installations
            .iter()
            .position(|record| record.spec.task == task)
        else {
            return Ok(());
        };
        self.retire_install(self.installations[at].id)
    }

    pub(crate) fn install_id(&self, task: TaskId) -> Option<InstallId> {
        self.installations
            .iter()
            .find(|record| record.spec.task == task)
            .map(|record| record.id)
    }

    pub(crate) fn retire_install(&mut self, id: InstallId) -> Result<(), &'static str> {
        let Some(at) = self.installations.iter().position(|record| record.id == id) else {
            return Ok(());
        };
        let Some(installer) = self.installer.as_ref() else {
            return Ok(());
        };
        let status = match installer.retire(id, Wait::AtMost(BOOT_MS)) {
            Ok(status) => status,
            Err(_) => installer
                .inspect(id, Wait::AtMost(BOOT_MS))
                .map_err(|_| "inspect identity retirement")?,
        };
        match status.state {
            InstallState::Retired | InstallState::Cancelled => {
                self.revoke_deliveries(id)?;
                self.installations.remove(at);
                Ok(())
            }
            InstallState::Unseen => Err("identity installation unseen"),
            _ => Err("identity retirement pending"),
        }
    }

    fn ensure_installed(&self, task: TaskId) -> Result<InstallStatus, &'static str> {
        let record = self
            .installations
            .iter()
            .find(|record| record.spec.task == task)
            .ok_or("identity installation missing")?;
        let installer = self.installer.as_ref().ok_or("identity not installed")?;
        let status = match installer.inspect(record.id, Wait::AtMost(BOOT_MS)) {
            Ok(status) if status.state == InstallState::Unseen => {
                self.retry_install(record.id, record.spec)?
            }
            Ok(status) => status,
            Err(_) => return Err("inspect identity installation"),
        };
        Ok(status)
    }

    fn require_installed(&self, status: InstallStatus) -> Result<(), &'static str> {
        match status.state {
            InstallState::Installed => Ok(()),
            InstallState::Unseen => Err("identity installation unseen"),
            InstallState::Failed => Err("identity installation failed"),
            _ => Err("identity installation pending"),
        }
    }
}

pub(crate) fn install(
    roster: &mut Roster,
    operator: TaskId,
    authority: TaskId,
) -> Result<[PieToken; Grant::ALL.len()], &'static str> {
    roster.retire()?;
    let mut faces = [None; Grant::ALL.len()];
    for grant in Grant::ALL {
        let token = face_of(authority, grant)?;
        faces[grant.index()] = Some(token);
    }
    let face = |grant: Grant| faces[grant.index()].ok_or("identity face missing");
    let installer =
        Installer::direct_with_install(authority, face(Grant::Install)?, face(Grant::Derive)?)
            .map_err(|_| "identity installer source")?;
    roster.installer = Some(installer);
    let root = PrincipalId::root(authority);
    let mut control = None;
    let mut installed = alloc::vec::Vec::new();
    for task in [Some(unit::self_id()), Some(operator), Some(authority)]
        .into_iter()
        .flatten()
    {
        let (id, status) = roster
            .install_status(task, Install::Derive { parent: root })
            .map_err(|_| "bootstrap identity install unknown")?;
        roster
            .require_installed(status)
            .map_err(|_| "bootstrap identity pending")?;
        let principal = status
            .binding
            .ok_or("bootstrap identity binding missing")?
            .current
            .principal;
        installed.push((task, id));
        if task == unit::self_id() {
            control = Some(principal);
        }
    }
    let control = control.ok_or("control identity missing")?;
    roster.control = Some(control);
    for (task, _) in installed {
        if task != unit::self_id() {
            roster.inject(task)?;
        }
    }
    Ok(core::array::from_fn(|i| faces[i].unwrap()))
}

mod query {
    use super::Roster;
    use crate::support::timing::BOOT_MS;
    use env::{TaskId, Wait};
    use ipc::session::establish;
    use system_api::identity::Wire;
    use system_api::identity::Grant;
    use system_api::operator::Fail;
    use system_client::identity::Face;

    pub(crate) fn current_authority(roster: &Roster) -> Option<TaskId> {
        roster
            .authority()
            .filter(|authority| !env::unit::join_task(*authority, Wait::POLL).unwrap_or(true))
    }
    fn face(roster: &Roster, grant: Grant) -> Result<Face, Fail> {
        let authority = roster.authority().ok_or(Fail::Unjudged)?;
        let entry = establish::find(authority, grant.mark()).map_err(|failure| match failure {
            establish::DiscoveryFail::Missing => Fail::Unjudged,
            establish::DiscoveryFail::Ambiguous => Fail::Denied,
        })?;
        Face::direct(authority, grant, entry).map_err(|_| Fail::Unjudged)
    }
    pub(crate) fn binding(
        roster: &Roster,
        task: TaskId,
    ) -> Result<Option<system_api::identity::Binding>, Fail> {
        match face(roster, Grant::Resolve)?
            .call(Wire::Resolve(task), Wait::AtMost(BOOT_MS))
            .map_err(|_| Fail::Unjudged)?
        {
            system_api::identity::Reply::Binding(b) => Ok(b),
            _ => Err(Fail::Unjudged),
        }
    }
}

pub(crate) use query::{binding, current_authority};

fn face_of(authority: TaskId, grant: Grant) -> Result<PieToken, &'static str> {
    let mut left = BOOT_MS;
    loop {
        let entry = match establish::find(authority, grant.mark()) {
            Ok(entry) => entry,
            Err(establish::DiscoveryFail::Ambiguous) => {
                return Err("identity face ambiguous");
            }
            Err(establish::DiscoveryFail::Missing) => {
                if left == 0 {
                    return Err("identity face missing");
                }
                execution::room::park(Duration::from_millis(RETRY_MS as u64))
                    .map_err(|_| "identity wait")?;
                left = left.saturating_sub(RETRY_MS);
                continue;
            }
        };
        if matches!(reserve(entry), Ok((_, owner, mark))
            if owner == authority && mark == grant.mark())
        {
            return Ok(entry);
        }
        return Err("identity face source");
    }
}

fn carrier_permission(root: Permission) -> Permission {
    Permission::STORE | Permission::VEST | (root & Permission::ONLY)
}

impl Roster {
    pub(crate) fn allow_subject(&self, from: TaskId, subject: Subject) -> Result<(), &'static str> {
        use system_api::identity::{Reply, Wire};
        use system_client::identity::Face;
        let authority = current_authority(self).ok_or("identity authority stale")?;
        let binding = binding(self, from)
            .map_err(|_| "creator unbound")?
            .ok_or("creator unbound")?;
        if subject.principal.authority != authority
            || !subject.coalitions.is_subset(&binding.current.coalitions)
        {
            return Err("creation subject outside grant");
        }
        let face = Face::direct(authority, Grant::Heir, face_of(authority, Grant::Heir)?)
            .map_err(|_| "creation identity face")?;
        if face
            .call(
                Wire::Heir(binding.current.principal, subject.principal),
                Wait::AtMost(BOOT_MS),
            )
            .map_err(|_| "creation identity query")?
            != Reply::Bool(true)
        {
            return Err("creation subject outside subtree");
        }
        Ok(())
    }
}
