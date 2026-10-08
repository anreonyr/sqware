//! Publication target vocabulary and fixed wire representation.

use crate::identity::{CoalitionId, PrincipalId};
use crate::operator::path::{Path, PathBuf};
use crate::operator::{EntryId, Fail, Permit};
use alloc::string::String;
use env::wire::Span as _;
use env::{PieToken, TaskId};
use wire::message::Message;

pub use super::marks::IDENTITY_REF as REF;
pub use super::marks::PUBLICATION_BACK as BACK;
pub use super::marks::PUBLICATION_ENTRY as ENTRY;
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

pub use crate::operator::name::valid as valid_name;

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
    pub fn new(op: u8, target: Target, resource: (PieToken, Permit)) -> Self {
        let (entry, permit) = resource;
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
        let (frame, end) = Self::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(frame)
    }
}
impl Message for Frame {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
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
            status: crate::operator::frame::fail_to_code(Some(fail)),
            kind: 0,
            task: TaskId::new(0),
            number: 0,
        }
    }
    pub fn result(self) -> Result<Self, Fail> {
        if self.status == 0 {
            Ok(self)
        } else {
            Err(crate::operator::frame::code_to_fail(self.status).unwrap_or(Fail::Unknown))
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
impl Message for Reply {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != Self::LEN {
            return None;
        }
        Self::fetch_at(bytes, 0).map(|one| one.0)
    }
}

fn reply_to(request: &Frame) -> env::PieToken {
    request.back
}

#[mold::contract(
    request = Frame,
    response = Reply,
    mark = super::interface::PUBLICATION_BACK,
    back = reply_to
)]
pub struct Call;
