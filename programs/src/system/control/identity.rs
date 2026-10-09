//! Control owns identity installation; bootstrap is direct IPC, never Operator discovery.
//!
//! Static units are explicitly authorized with separate root-derived principals. Runtime
//! children inherit the kernel sender's current snapshot before embark. An unavailable
//! authority is not a reason to release a child without an identity.

use ::resource::port;
use env::unit;
use env::{Access, PieToken, Policy, TaskId, Wait};
use system_api::identity::Grant;
use system_api::identity::Install;
use system_api::identity::PrincipalId;
use system_api::identity::Subject;
use system_client::identity::Installer;

use crate::support::timing::{BOOT_MS, RETRY_MS};
use core::time::Duration;
use ipc::session::establish;
use resource::raw::reserve;

const _: () = assert!(Grant::ALL.len() <= system_api::operator::frame::PANE_CAP);

#[derive(Default)]
pub(crate) struct Roster {
    installer: Option<Installer>,
    control: Option<PrincipalId>,
    resolve: Option<PieToken>,
}

impl Roster {
    pub(crate) fn retire(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn control(&self) -> Option<PrincipalId> {
        self.control
    }

    pub(crate) fn authority(&self) -> Option<TaskId> {
        self.installer.as_ref().map(Installer::authority)
    }

    /// Give an external unit its own root-derived principal before embark.
    pub(crate) fn authorize(&self, task: TaskId) -> Result<(), &'static str> {
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

    pub(crate) fn install(&self, task: TaskId, identity: Install) -> Result<(), &'static str> {
        self.installer
            .as_ref()
            .ok_or("identity not installed")?
            .bind(task, identity, Wait::AtMost(BOOT_MS))
            .map_err(|_| "install task identity")?;
        self.inject(task)
    }

    pub(crate) fn inherit(&self, task: TaskId, parent: TaskId) -> Result<(), &'static str> {
        self.install(task, Install::Inherit { parent })
    }

    fn inject(&self, task: TaskId) -> Result<(), &'static str> {
        let resolve = self.resolve.ok_or("identity authority anchor")?;
        port::ship(resolve, task, Access::STORE | Access::FETCH, Policy::NONE)
            .map(|_| ())
            .map_err(|_| "inject identity authority")
    }

    pub(crate) fn unbind(&self, task: TaskId) -> Result<(), &'static str> {
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

mod query {
    use super::Roster;
    use crate::support::timing::BOOT_MS;
    use env::{TaskId, Wait};
    use ipc::session::establish;
    use system_api::control::Object;
    use system_api::identity::Grant;
    use system_api::identity::Selector;
    use system_api::identity::Wire;
    use system_api::operator::Fail;
    use system_api::operator::Permit;
    use system_client::identity::Face;

    pub(crate) fn current_authority(roster: &Roster) -> Option<TaskId> {
        roster
            .authority()
            .filter(|authority| !env::unit::join(*authority, Wait::POLL).unwrap_or(true))
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
    pub(crate) fn validate(roster: &Roster, object: Object) -> Result<(), Fail> {
        if roster.authority() != Some(object.authority()) {
            return Err(Fail::Unjudged);
        }
        let (grant, wire) = match object {
            Object::Principal(p) => (Grant::Heir, Wire::Heir(p, p)),
            Object::Coalition(c) => (Grant::Members, Wire::Members(c, None)),
        };
        face(roster, grant)?
            .call(wire, Wait::AtMost(BOOT_MS))
            .map(|_| ())
            .map_err(|_| Fail::Unjudged)
    }
    pub(crate) fn validate_permit(roster: &Roster, permit: Permit) -> Result<(), Fail> {
        match permit {
            Permit::Identity(Selector::Exact(p) | Selector::DescendantOf(p)) => {
                validate(roster, Object::Principal(p))
            }
            Permit::Identity(Selector::MemberOf(c)) => validate(roster, Object::Coalition(c)),
            _ => Ok(()),
        }
    }
}

pub(crate) use query::{binding, current_authority, validate, validate_permit};

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

impl Roster {
    pub(crate) fn allow_subject(&self, from: TaskId, subject: Subject) -> Result<(), &'static str> {
        use system_api::identity::{Reply, Wire};
        use system_client::identity::Face;
        let authority = current_authority(self).ok_or("identity authority stale")?;
        let binding = binding(self, from).map_err(|_| "creator unbound")?.ok_or("creator unbound")?;
        if subject.principal.authority != authority || !subject.coalitions.is_subset(&binding.current.coalitions) {
            return Err("creation subject outside grant");
        }
        let face = Face::direct(authority, Grant::Heir, face_of(authority, Grant::Heir)?).map_err(|_| "creation identity face")?;
        if face.call(Wire::Heir(binding.current.principal, subject.principal), Wait::AtMost(BOOT_MS)).map_err(|_| "creation identity query")? != Reply::Bool(true) {
            return Err("creation subject outside subtree");
        }
        Ok(())
    }
}
