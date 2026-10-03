//! Control owns identity installation; bootstrap is direct IPC, never Operator discovery.
//!
//! Static units are explicitly authorized with separate root-derived principals. Runtime
//! children inherit the kernel sender's current snapshot before hatch. An unavailable
//! authority is not a reason to release a child without an identity.

use core::time::Duration;

use env::{Access, PieToken, Policy, TaskId, Wait};
use protocol::communication::session::establish;
use protocol::service::identity::client::Installer;
use protocol::service::identity::{Grant, Install, PrincipalId, Subject};
use protocol::service::operator::Permit;
use runtime::core::res::port;
use runtime::env::{mail, room, unit};

use crate::system::Assembly;
use crate::system::control::{BOOT_MS, RETRY_MS, Service};
use crate::unit::UnitFile;

const _: () = assert!(Grant::ALL.len() <= protocol::service::operator::frame::PANE_CAP);

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

    /// The only pre-identity exceptions are the two paused-bootstrap authorities.
    pub fn authorize(&self, task: TaskId, program: &UnitFile) -> Result<(), &'static str> {
        let Some(installer) = self.installer.as_ref() else {
            return if matches!(program.name(), "operator" | "identity") {
                Ok(())
            } else {
                Err("identity not installed")
            };
        };
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
        coalition: protocol::service::identity::CoalitionId,
    ) -> Result<(), &'static str> {
        use protocol::service::identity::client::Face;
        use protocol::service::identity::{Reply, Wire, limits::MAX_ACTIVE_COALITIONS};

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

pub fn bind(
    assembly: &mut Assembly,
    program: &UnitFile,
    service: &mut Service,
) -> Result<(), &'static str> {
    assembly.control.authorize_static(service.0, program)
}

/// Readiness of this system unit means the complete source-checked bundle is published,
/// not merely that the local Identity loop can respond.
pub fn adopt(
    assembly: &mut Assembly,
    program: &UnitFile,
    service: &mut Service,
) -> Result<(), &'static str> {
    if program.name() != "identity" {
        return Ok(());
    }
    install(&mut assembly.control, &mut assembly.tree, service.0)
}

pub(crate) fn install(
    control_state: &mut crate::system::control::Control,
    tree: &mut crate::service::operator::bridge::Tree,
    authority: TaskId,
) -> Result<(), &'static str> {
    control_state.roster.retire();
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
    for task in [Some(unit::self_id()), tree.host(), Some(authority)]
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
    tree.wire(
        authority,
        face(Grant::Resolve)?,
        face(Grant::Matches)?,
        face(Grant::Same)?,
    )?;
    for grant in Grant::ALL {
        let permit = match grant.mount() {
            protocol::service::identity::Mount::Public => Permit::Public,
            protocol::service::identity::Mount::Bound => Permit::Bound,
            protocol::service::identity::Mount::Installer => {
                Permit::Identity(protocol::service::identity::Selector::Exact(control))
            }
        };
        let road = protocol::service::identity::DIR
            .try_join(grant.name())
            .ok_or("identity face path")?;
        control_state.hierarchy.borrow_mut().internal(
            tree,
            road.as_path(),
            face(grant)?,
            permit,
            authority,
            Some(authority),
        )?;
    }
    control_state.roster = Roster {
        installer: Some(installer),
        control: Some(control),
        resolve: Some(face(Grant::Resolve)?),
    };
    control_state.progress(tree)?;
    let principal = control_state.roster.control().ok_or("control identity")?;
    control_state.hierarchy.borrow_mut().register(
        control_state,
        tree,
        "control",
        protocol::system::control::publication::Object::Principal(principal),
        None,
    )?;
    protocol::debug::put("system: identity installed; 17 source-checked faces published");
    Ok(())
}

/// Obtain the startup authority from a Control-injected face, not a marked public entry.
/// Kernel Sire and vestor anchor the trust decision; the original owner is the Identity
/// instance. Discovery clients then verify every other face against that exact owner.
pub fn authority() -> Option<TaskId> {
    let sire = unit::sire();
    let mut authority = None;
    for pie in mail::pies() {
        if pie.mark != Grant::Resolve.mark() {
            continue;
        }
        let Ok((vestor, owner, mark)) = mail::reserve(pie.token) else {
            continue;
        };
        if vestor != sire || owner == sire || mark != Grant::Resolve.mark() {
            continue;
        }
        if authority.is_some_and(|known| known != owner) {
            return None;
        }
        authority = Some(owner);
    }
    authority
}

fn face_of(authority: TaskId, grant: Grant) -> Result<PieToken, &'static str> {
    let mut left = BOOT_MS;
    loop {
        if let Some(entry) = establish::find(authority, grant.mark()) {
            if matches!(mail::reserve(entry), Ok((_, owner, mark))
                if owner == authority && mark == grant.mark())
            {
                return Ok(entry);
            }
            return Err("identity face source");
        }
        if left == 0 {
            return Err("identity face missing");
        }
        room::sleep(Duration::from_millis(RETRY_MS as u64)).map_err(|_| "identity wait")?;
        left = left.saturating_sub(RETRY_MS);
    }
}
