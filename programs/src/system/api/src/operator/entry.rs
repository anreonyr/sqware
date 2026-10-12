use super::{Entry, EntryId, EntryTarget, Fail, GrantId, GrantState, Permit};
use alloc::string::String;
use env::{PieToken, TaskId};
use wire::Message;
use wire::Span as _;

pub use super::interface::{AUTH_BACK, AUTH_ENTRY};
pub const PART: u8 = 1;
pub const BIND: u8 = 2;
pub const REPLACE: u8 = 3;
pub const UNBIND: u8 = 4;
pub const GRANT: u8 = 5;
pub const INSPECT: u8 = 6;
pub const REVOKE: u8 = 7;
pub const UNPART: u8 = 8;
pub const FIND: u8 = 9;
pub const READ: u8 = 10;
pub const CANCEL_OPERATION: u8 = 11;
pub const INSPECT_OPERATION: u8 = 12;
pub const ASSOCIATE: u8 = 13;
pub const RESOLVE_NAMESPACE: u8 = 14;

#[derive(env::Frame, Clone, Debug, PartialEq, Eq)]
#[frame(len = 512)]
pub struct Request {
    pub op: u8,
    pub anchor: EntryId,
    pub root: u8,
    pub id: u64,
    pub parent: u64,
    pub holder: TaskId,
    pub ops: u64,
    pub permit: Permit,
    pub permit_restricted: u8,
    pub source: TaskId,
    pub source_mark: u64,
    pub resource: PieToken,
    pub resource_id: u64,
    pub view: u32,
    pub expected: u64,
    pub name: String,
    pub scope: String,
    pub back: PieToken,
}
impl Message for Request {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        let (request, end) = Self::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(request)
    }
}

#[derive(env::Frame, Clone, Copy, Debug, PartialEq, Eq)]
#[frame(len = 128)]
pub struct Response {
    pub status: u8,
    pub state: u8,
    pub root: u8,
    pub entry: EntryId,
    pub revision: u64,
    pub grant: u64,
    pub token: PieToken,
    pub view: u32,
    pub permit: super::Permit,
}
impl Message for Response {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        let (response, end) = Self::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(response)
    }
}

pub fn reply_to(request: &Request) -> PieToken {
    request.back
}

#[mold::contract(request = Request, response = Response, mark = AUTH_BACK, back = reply_to)]
pub struct Call;

impl Response {
    pub fn fail(fail: Fail) -> Self {
        Self {
            status: super::frame::fail_to_code(Some(fail)),
            state: 0,
            root: 0,
            entry: EntryId::new(0),
            revision: 0,
            grant: 0,
            token: PieToken::NONE,
            view: 0,
            permit: super::Permit::Public,
        }
    }
    pub fn entry(self, operator: TaskId) -> Result<Entry, Fail> {
        if self.status != super::OK {
            return Err(super::frame::code_to_fail(self.status).unwrap_or(Fail::Unknown));
        }
        Ok(Entry {
            operator,
            target: if self.root == 1 {
                EntryTarget::Root
            } else {
                EntryTarget::Node(self.entry)
            },
        })
    }
    pub fn grant_state(self) -> Result<GrantState, Fail> {
        if self.status != super::OK {
            return Err(super::frame::code_to_fail(self.status).unwrap_or(Fail::Unknown));
        }
        match self.state {
            0 => Ok(GrantState::Unseen),
            1 => Ok(GrantState::Pending),
            2 => Ok(GrantState::Granted),
            3 => Ok(GrantState::Revoking),
            4 => Ok(GrantState::Closed),
            _ => Err(Fail::Unknown),
        }
    }
    pub fn grant_id(self) -> GrantId {
        GrantId(self.grant)
    }
    pub fn operation_state(self) -> Result<super::OperationState, Fail> {
        if self.status != super::OK {
            return Err(super::frame::code_to_fail(self.status).unwrap_or(Fail::Unknown));
        }
        match self.state {
            0 => Ok(super::OperationState::Unseen),
            1 => Ok(super::OperationState::Pending),
            2 => Ok(super::OperationState::Applied),
            3 => Ok(super::OperationState::Closed),
            4 => Ok(super::OperationState::Failed),
            _ => Err(Fail::Unknown),
        }
    }
}
