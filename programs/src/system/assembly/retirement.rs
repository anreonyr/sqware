use system_api::retirement::Call;
use system_client::retirement::Face;

/// One declared retirement provider and its in-flight request.
pub(crate) struct Participant {
    pub(crate) declaration: crate::unit::Retirement,
    target: env::TaskId,
    pub(crate) face: Option<Face>,
    pub(crate) pending: Option<ipc::rpc::request::Pending<Call>>,
    pub(crate) done: bool,
}

impl Participant {
    pub(crate) fn new(declaration: crate::unit::Retirement, target: env::TaskId) -> Self {
        Self {
            declaration,
            target,
            face: None,
            pending: None,
            done: false,
        }
    }

    pub(crate) fn discover(
        &mut self,
        system: &crate::system::app::namespace::System,
        namespaces: &crate::system::assembly::namespace::NamespaceInstallations,
    ) {
        if self.done || self.face.is_some() || self.pending.is_some() {
            return;
        }
        let Some(grant) = namespaces.program_grant(
            self.declaration.provider, self.declaration.namespace,
        ) else { return };
        let Ok(bound) = system.capability().find(
            grant.entry, self.declaration.face, system_api::operator::Permit::Public,
            env::Wait::AtMost(MS),
        ) else { return };
        if let Ok(face) = Face::of(bound.resource) {
            self.face = Some(face);
        }
    }

    pub(crate) fn advance(&mut self) {
        let target = self.target;
        if self.done { return; }
        if let Some(mut pending) = self.pending.take() {
            match pending.poll() {
                Ok(Some(reply)) if reply.target == target && reply.settled() => self.done = true,
                Ok(Some(_)) => {}
                Ok(None) => self.pending = Some(pending),
                Err(_) => self.face = None,
            }
            return;
        }
        let Some(begin) = self.face.as_ref()
            .map(|face| face.begin_for(target, env::Wait::AtMost(MS)))
        else { return };
        match begin {
            Ok(mut pending) => match pending.poll() {
                Ok(Some(reply)) if reply.target == target && reply.settled() => self.done = true,
                Ok(Some(_)) => {}
                Ok(None) => self.pending = Some(pending),
                Err(_) => self.face = None,
            },
            Err(_) => self.face = None,
        }
    }
}

const MS: usize = 1000;
