//! Control owns publication policy; clients submit typed objects, never absolute paths.
use crate::common::path::{Path, PathBuf};
use crate::communication::hand::Sender;
use crate::communication::session::establish;
use crate::system::identity::{CoalitionId, PrincipalId};
use crate::system::operator::{EntryId, Fail, Permit};
use crate::wire::message::Message;
use alloc::string::String;
use env::wire::Span as _;
use env::{PieToken, TaskId, Wait};
use env::pie;
use ::resource::raw::{Hole, inspect, reserve};

pub use super::marks::PUBLICATION_ENTRY as ENTRY;
pub use super::marks::PUBLICATION_BACK as BACK;
pub use super::marks::IDENTITY_REF as REF;
pub const PUBLISH: u8 = 1;
pub const UNPUBLISH: u8 = 2;
pub const RESOLVE: u8 = 3;
pub const RUNTIME: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Driver = 1,
    Hub = 2,
    Device = 3,
    Fixture = 4,
    Terminal = 5,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Service {
        scope: Scope,
        group: String,
        name: String,
    },
    IdentityName {
        object: Object,
        name: String,
    },
    RuntimeResource {
        task: TaskId,
        kind: String,
        name: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Object {
    Principal(PrincipalId),
    Coalition(CoalitionId),
}
impl Object {
    pub fn authority(self) -> TaskId {
        match self {
            Self::Principal(p) => p.authority,
            Self::Coalition(c) => c.authority,
        }
    }
    pub fn slot(self) -> u64 {
        match self {
            Self::Principal(p) => p.slot,
            Self::Coalition(c) => c.slot,
        }
    }
    pub fn kind(self) -> u8 {
        match self {
            Self::Principal(_) => 1,
            Self::Coalition(_) => 2,
        }
    }
    pub fn road(self, name: &str) -> Option<PathBuf> {
        if !valid_name(name) {
            return None;
        }
        let base = match self {
            Self::Principal(_) => Path::new("idt/principal"),
            Self::Coalition(_) => Path::new("idt/coalition"),
        };
        base.try_join(name)?.try_join("ref")
    }
}

pub use crate::common::name::valid as valid_name;

#[derive(env::Frame, Clone, Debug, PartialEq, Eq)]
#[frame(len = 160)]
pub struct Frame {
    pub op: u8,
    pub kind: u8,
    pub group: String,
    pub name: String,
    pub task: TaskId,
    pub number: u64,
    pub entry: PieToken,
    pub permit: Permit,
    pub back: PieToken,
}
impl Frame {
    pub fn new(op: u8, target: Target, entry: PieToken, permit: Permit) -> Self {
        let (kind, group, name, task, number) = match target {
            Target::Service { scope, group, name } => (scope as u8, group, name, TaskId::new(0), 0),
            Target::IdentityName { object, name } => (
                10 + object.kind(),
                String::new(),
                name,
                object.authority(),
                object.slot(),
            ),
            Target::RuntimeResource { task, kind, name } => (20, kind, name, task, 0),
        };
        Self {
            op,
            kind,
            group,
            name,
            task,
            number,
            entry,
            permit,
            back: PieToken::NONE,
        }
    }
    pub fn target(&self) -> Option<Target> {
        if !valid_name(&self.name) || (!self.group.is_empty() && !valid_name(&self.group)) {
            return None;
        }
        let object = match self.kind {
            11 => Some(Object::Principal(PrincipalId::new(self.task, self.number))),
            12 => Some(Object::Coalition(CoalitionId::new(self.task, self.number))),
            _ => None,
        };
        if let Some(object) = object {
            if !self.group.is_empty() {
                return None;
            }
            return Some(Target::IdentityName {
                object,
                name: self.name.clone(),
            });
        }
        if self.kind == 20 {
            if self.number != 0 || self.group.is_empty() || self.task.get() == 0 {
                return None;
            }
            return Some(Target::RuntimeResource {
                task: self.task,
                kind: self.group.clone(),
                name: self.name.clone(),
            });
        }
        let scope = match self.kind {
            1 => Scope::Driver,
            2 => Scope::Hub,
            3 => Scope::Device,
            4 => Scope::Fixture,
            5 => Scope::Terminal,
            _ => return None,
        };
        (self.task.get() == 0 && self.number == 0).then(|| Target::Service {
            scope,
            group: self.group.clone(),
            name: self.name.clone(),
        })
    }
    pub fn take(bytes: &[u8]) -> Option<Self> {
        let (frame, at) = Self::fetch_at(bytes, 0)?;
        (at == bytes.len()).then_some(frame)
    }
}
impl Message for Frame {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];
    fn store(&self, bytes: &mut [u8]) -> Option<usize> {
        self.store_at(bytes, 0)
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        Self::take(bytes)
    }
}

#[derive(env::Frame, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reply {
    pub status: u8,
    pub kind: u8,
    pub task: TaskId,
    pub number: u64,
}
impl Reply {
    pub fn from_sender(control: TaskId, from: TaskId, bytes: &[u8]) -> Result<Self, Fail> {
        if bytes.len() != Self::LEN || from != control {
            return Err(Fail::Denied);
        }
        Self::fetch_at(bytes, 0)
            .map(|one| one.0)
            .ok_or(Fail::Unknown)
    }
    pub fn mount(id: EntryId) -> Self {
        Self {
            status: 0,
            kind: 0,
            task: TaskId::new(0),
            number: id.get() as u64,
        }
    }
    pub fn object(object: Object) -> Self {
        Self {
            status: 0,
            kind: object.kind(),
            task: object.authority(),
            number: object.slot(),
        }
    }
    pub fn fail(fail: Fail) -> Self {
        Self {
            status: crate::system::operator::fail_to_code(Some(fail)),
            kind: 0,
            task: TaskId::new(0),
            number: 0,
        }
    }
    pub fn result(self) -> Result<Self, Fail> {
        if self.status == 0 {
            Ok(self)
        } else {
            Err(crate::system::operator::code_to_fail(self.status).unwrap_or(Fail::Unknown))
        }
    }
    pub fn identity(self, authority: TaskId) -> Result<Object, Fail> {
        let reply = self.result()?;
        if reply.task != authority {
            return Err(Fail::Unjudged);
        }
        match reply.kind {
            1 => Ok(Object::Principal(PrincipalId::new(
                reply.task,
                reply.number,
            ))),
            2 => Ok(Object::Coalition(CoalitionId::new(
                reply.task,
                reply.number,
            ))),
            _ => Err(Fail::Unknown),
        }
    }
}

#[derive(Clone, Copy)]
pub struct Client {
    control: TaskId,
    entry: PieToken,
}
impl Client {
    pub fn direct(control: TaskId, entry: PieToken) -> Result<Self, Fail> {
        if !matches!(reserve(entry), Ok((_, owner, mark)) if owner == control && mark == ENTRY)
        {
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
        let mut frame = Frame::new(PUBLISH, target, entry, permit);
        if frame.target().is_none() {
            return Err(Fail::Denied);
        }
        Self::direct(self.control, self.entry)?;
        if !matches!(inspect(entry), Ok((_, owner, _)) if owner == env::unit::self_id())
        {
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
            Frame::new(UNPUBLISH, target, PieToken::NONE, Permit::Public),
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
            PieToken::NONE,
            Permit::Public,
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
            PieToken::NONE,
            Permit::Public,
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
    let (back, seed) = establish::lend_out(entry, BACK).map_err(|_| Fail::Unknown)?;
    struct Back(PieToken);
    impl Drop for Back {
        fn drop(&mut self) {
            let _ = pie::seal(self.0);
            let _ = pie::release(self.0);
        }
    }
    let _back = Back(back);
    frame.back = seed;
    Sender::<Frame>::from_raw(entry)
        .send_within(frame.clone(), wait)
        .map_err(|_| Fail::Unknown)?;
    *admitted = true;
    let mut bytes = [0; Reply::LEN];
    let (n, from) = Hole::from_raw(back)
        .pull(&mut bytes, wait)
        .map_err(|_| Fail::Unknown)?;
    Reply::from_sender(control, from, &bytes[..n])
}
