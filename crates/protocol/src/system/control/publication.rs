//! Control publication client. Wire types live in system-api.

use crate::common::path::PathBuf;
use crate::system::identity::{CoalitionId, PrincipalId};
use crate::system::operator::Fail;
use crate::system::operator::{EntryId, Permit};
use ::resource::raw::{inspect, reserve};
use env::pie;
use env::{PieToken, TaskId, Wait};
use ipc::session::establish;
use ipc::{rpc, time::Deadline};

pub use system_api::control::publication::{
    BACK, ENTRY, Frame, Object, PUBLISH, REF, RESOLVE, RUNTIME, Reply, Scope, Target, UNPUBLISH,
    valid_name,
};

#[derive(Clone, Copy)]
pub struct Client {
    control: TaskId,
    entry: PieToken,
}
impl Client {
    pub fn direct(control: TaskId, entry: PieToken) -> Result<Self, Fail> {
        if !matches!(reserve(entry), Ok((_, owner, mark)) if owner == control && mark == ENTRY) {
            return Err(Fail::Denied);
        }
        Ok(Self { control, entry })
    }
    pub fn injected() -> Result<Self, Fail> {
        let control = env::unit::sire();
        let entry = establish::find(control, ENTRY).ok_or(Fail::Unknown)?;
        if !matches!(reserve(entry), Ok((vestor, owner, _)) if vestor == control && owner == control)
        {
            return Err(Fail::Denied);
        }
        Self::direct(control, entry)
    }
    pub fn call(&self, mut frame: Frame, wait: Wait) -> Result<Reply, Fail> {
        Self::direct(self.control, self.entry)?;
        exchange(self.control, self.entry, &mut frame, wait, &mut false)?.result()
    }
    pub fn publish(
        &self,
        target: Target,
        entry: PieToken,
        permit: Permit,
        wait: Wait,
    ) -> Result<EntryId, Fail> {
        let mut frame = Frame::new(PUBLISH, target, (entry, permit));
        if frame.target().is_none() {
            return Err(Fail::Denied);
        }
        Self::direct(self.control, self.entry)?;
        if !matches!(inspect(entry), Ok((_, owner, _)) if owner == env::unit::self_id()) {
            return Err(Fail::Denied);
        }
        let seed = ::resource::port::ship(
            entry,
            self.control,
            ::resource::port::Access::FETCH | ::resource::port::Access::STORE,
            ::resource::port::Policy::VEST,
        )
        .map_err(|_| Fail::Denied)?
        .seed();
        frame.entry = seed;
        let mut admitted = false;
        let result = exchange(self.control, self.entry, &mut frame, wait, &mut admitted)
            .and_then(Reply::result)
            .map(|r| EntryId::new(r.number as usize));
        // Once admitted, timeout is an unknown outcome and does not cancel publication.
        if result.is_err() && !admitted {
            let _ = pie::revoke(self.control, seed);
        }
        result
    }

    pub fn unpublish(&self, target: Target, wait: Wait) -> Result<(), Fail> {
        self.call(
            Frame::new(UNPUBLISH, target, (PieToken::NONE, Permit::Public)),
            wait,
        )
        .map(|_| ())
    }
    pub fn runtime(&self, task: TaskId, wait: Wait) -> Result<PathBuf, Fail> {
        let frame = Frame::new(
            RUNTIME,
            Target::RuntimeResource {
                task,
                kind: "hole".into(),
                name: "directory".into(),
            },
            (PieToken::NONE, Permit::Public),
        );
        let reply = self.call(frame, wait)?;
        PathBuf::try_new(&alloc::format!("uit/{}/{}", reply.number, reply.task.get()))
            .ok_or(Fail::Unknown)
    }
    pub fn reference(
        &self,
        operator: &crate::system::operator::client::Face,
        authority: TaskId,
        kind: u8,
        name: &str,
        wait: Wait,
    ) -> Result<Object, Fail> {
        let object = match kind {
            1 => Object::Principal(PrincipalId::root(authority)),
            2 => Object::Coalition(CoalitionId::root(authority)),
            _ => return Err(Fail::Unknown),
        };
        let road = object.road(name).ok_or(Fail::Unknown)?;
        let entry = operator.root().tile(&road, wait)?.token(wait)?;
        Self::reference_direct(self.control, authority, entry, kind, name, wait)
    }
    pub fn reference_direct(
        control: TaskId,
        authority: TaskId,
        entry: PieToken,
        kind: u8,
        name: &str,
        wait: Wait,
    ) -> Result<Object, Fail> {
        if !valid_name(name)
            || !matches!(reserve(entry), Ok((_, owner, mark)) if owner == control && mark == REF)
        {
            return Err(Fail::Denied);
        }
        let object = match kind {
            1 => Object::Principal(PrincipalId::root(authority)),
            2 => Object::Coalition(CoalitionId::root(authority)),
            _ => return Err(Fail::Unknown),
        };
        let mut frame = Frame::new(
            RESOLVE,
            Target::IdentityName {
                object,
                name: name.into(),
            },
            (PieToken::NONE, Permit::Public),
        );
        let reply = exchange(control, entry, &mut frame, wait, &mut false)?;
        if reply.status == 0 && reply.kind != kind {
            return Err(Fail::Denied);
        }
        reply.identity(authority)
    }
}

fn exchange(
    control: TaskId,
    entry: PieToken,
    frame: &mut Frame,
    wait: Wait,
    admitted: &mut bool,
) -> Result<Reply, Fail> {
    let sender = rpc::request::Sender::<super::rpc::Publication>::from_raw(entry)
        .map_err(|_| Fail::Unknown)?;
    if sender.peer() != control {
        return Err(Fail::Denied);
    }
    let response = sender
        .send(Deadline::new(wait), |back| {
            frame.back = back;
            frame.clone()
        })
        .map_err(|_| Fail::Unknown)?;
    *admitted = true;
    response.receive().map_err(|error| match error {
        rpc::Fail::WrongSource | rpc::Fail::Decode => Fail::Denied,
        _ => Fail::Unknown,
    })
}
