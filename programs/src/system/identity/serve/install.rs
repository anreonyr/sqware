//! Control owns identity installation; bootstrap is direct IPC, never Operator discovery.
//!
//! Static units are explicitly authorized with separate root-derived principals. Runtime
//! children inherit the kernel sender's current snapshot before hatch. An unavailable
//! authority is not a reason to release a child without an identity.

use env::{Access, PieToken, Policy, TaskId, Wait};
use protocol::system::identity::client::Installer;
use protocol::system::identity::{Grant, Install, PrincipalId, Subject};
use runtime::core::res::port;
use runtime::env::{mail, unit};

use super::source::face_of;
use crate::system::control::serve::start::BOOT_MS;

const _: () = assert!(Grant::ALL.len() <= protocol::system::operator::frame::PANE_CAP);

#[derive(Default)]
pub struct Roster {
    installer: Option<Installer>,
    control: Option<PrincipalId>,
    resolve: Option<PieToken>,
}

impl Roster {
    pub(crate) fn retire(&mut self) {
        *self = Self::default();
    }

    pub fn control(&self) -> Option<PrincipalId> {
        self.control
    }

    pub(crate) fn authority(&self) -> Option<TaskId> {
        self.installer.as_ref().map(Installer::authority)
    }

    /// Give an external unit its own root-derived principal before hatch.
    pub fn authorize(&self, task: TaskId) -> Result<(), &'static str> {
        let installer = self.installer.as_ref().ok_or("identity not installed")?;
        let root = PrincipalId::root(installer.authority());
        let principal = installer
            .derive(root, Wait::AtMost(BOOT_MS))
            .map_err(|_| "derive unit identity")?;
        let subject = Subject::new(principal, &[]).map_err(|_| "unit subject")?;
        installer
            .bind(task, Install::Authorized(subject), Wait::AtMost(BOOT_MS))
            .map_err(|_| "authorize unit identity")?;
        self.inject(task)
    }

    pub fn inherit(&self, task: TaskId, parent: TaskId) -> Result<(), &'static str> {
        let installer = self.installer.as_ref().ok_or("identity not installed")?;
        installer
            .bind(task, Install::Inherit { parent }, Wait::AtMost(BOOT_MS))
            .map_err(|_| "inherit identity")?;
        self.inject(task)
    }

    fn inject(&self, task: TaskId) -> Result<(), &'static str> {
        let resolve = self.resolve.ok_or("identity authority anchor")?;
        port::ship(
            &mail::HolePie::from_token(resolve),
            task,
            Access::STORE | Access::FETCH,
            Policy::NONE,
        )
        .map(|_| ())
        .map_err(|_| "inject identity authority")
    }

    /// Explicit device grant, never a reset of a task's attenuated binding.
    pub(crate) fn activate(
        &self,
        task: TaskId,
        coalition: protocol::system::identity::CoalitionId,
    ) -> Result<(), &'static str> {
        use protocol::system::identity::client::Face;
        use protocol::system::identity::{Reply, Wire, limits::MAX_ACTIVE_COALITIONS};

        let installer = self.installer.as_ref().ok_or("identity not installed")?;
        let authority = installer.authority();
        let resolve = Face::direct(
            authority,
            Grant::Resolve,
            self.resolve.ok_or("identity authority anchor")?,
        )
        .map_err(|_| "identity resolve source")?;
        let Reply::Binding(Some(binding)) = resolve
            .call(Wire::Resolve(task), Wait::AtMost(BOOT_MS))
            .map_err(|_| "device identity resolve")?
        else {
            return Err("device identity unbound");
        };
        if binding.origin != binding.current || coalition.authority != authority {
            return Err("device identity narrowed or stale");
        }
        let subject = binding.current;
        let amid = Face::direct(authority, Grant::Amid, face_of(authority, Grant::Amid)?)
            .map_err(|_| "identity amid source")?;
        if amid
            .call(
                Wire::Amid(subject.principal, coalition),
                Wait::AtMost(BOOT_MS),
            )
            .map_err(|_| "device qualification query")?
            != Reply::Bool(true)
        {
            return Err("device identity not eligible");
        }
        if subject.coalitions.contains(coalition) {
            return Ok(());
        }
        let len = subject.coalitions.len();
        if len == MAX_ACTIVE_COALITIONS {
            return Err("device identity full");
        }
        let mut ids = [coalition; MAX_ACTIVE_COALITIONS];
        ids[..len].copy_from_slice(subject.coalitions.as_slice());
        let subject = Subject::new(subject.principal, &ids[..len + 1])
            .map_err(|_| "device identity subject")?;
        installer
            .bind(task, Install::Authorized(subject), Wait::AtMost(BOOT_MS))
            .map_err(|_| "device identity activate")
    }

    pub fn unbind(&self, task: TaskId) -> Result<(), &'static str> {
        let Some(installer) = self.installer.as_ref() else {
            return Ok(());
        };
        installer
            .unbind(task, Wait::AtMost(BOOT_MS))
            .map_err(|_| "unbind identity")
    }
}

pub(crate) fn install(
    roster: &mut Roster,
    operator: TaskId,
    authority: TaskId,
) -> Result<[PieToken; Grant::ALL.len()], &'static str> {
    roster.retire();
    let mut faces = [None; Grant::ALL.len()];
    for grant in Grant::ALL {
        let token = face_of(authority, grant)?;
        faces[grant.index()] = Some(token);
    }
    let face = |grant: Grant| faces[grant.index()].ok_or("identity face missing");
    let installer = Installer::direct(
        authority,
        face(Grant::Bind)?,
        face(Grant::Unbind)?,
        face(Grant::Derive)?,
    )
    .map_err(|_| "identity installer source")?;
    let root = PrincipalId::root(authority);
    let mut control = None;
    for task in [Some(unit::self_id()), Some(operator), Some(authority)]
        .into_iter()
        .flatten()
    {
        let principal = installer
            .derive(root, Wait::AtMost(BOOT_MS))
            .map_err(|_| "derive bootstrap identity")?;
        let subject = Subject::new(principal, &[]).map_err(|_| "bootstrap subject")?;
        installer
            .bind(task, Install::Authorized(subject), Wait::AtMost(BOOT_MS))
            .map_err(|_| "bind bootstrap identity")?;
        if task == unit::self_id() {
            control = Some(principal);
        }
    }
    let control = control.ok_or("control identity missing")?;
    *roster = Roster {
        installer: Some(installer),
        control: Some(control),
        resolve: Some(face(Grant::Resolve)?),
    };
    Ok(core::array::from_fn(|i| faces[i].unwrap()))
}
