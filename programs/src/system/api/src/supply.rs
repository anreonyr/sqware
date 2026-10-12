//! Host-safe declarations and wire contract for generic resource leases.

use env::{Mark, Permission, PieToken, TaskId};
use env::wire::Span as _;

pub const MAX_DESCRIPTION: usize = 128;
pub const MAX_LEASES: usize = 256;

/// A stable logical source or resource name. The composition root checks collisions.
#[derive(Clone, Copy, Debug)]
pub struct Key {
    name: &'static str,
    mark: Mark,
}

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool { self.mark == other.mark }
}
impl Eq for Key {}

impl Key {
    pub const fn of(name: &'static str) -> Self {
        Self { name, mark: Mark::of(name) }
    }

    pub const fn from_mark(mark: Mark) -> Self {
        Self { name: "", mark }
    }

    pub const fn mark(self) -> Mark {
        self.mark
    }

    pub const fn name(self) -> &'static str {
        self.name
    }
}

/// Caller-chosen, non-reusable operation identity.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LeaseId(pub u64);

/// Per-provider caller sequence. Reserve once before the first send and never reuse a value.
#[derive(Clone, Copy, Debug, Default)]
pub struct LeaseIds { next: u64 }

impl LeaseIds {
    pub const fn new() -> Self { Self { next: 0 } }

    pub fn reserve(&mut self) -> Result<LeaseId, Fail> {
        self.next = self.next.checked_add(1).ok_or(Fail::Full)?;
        Ok(LeaseId(self.next))
    }
}

/// One declared binding from a logical provider resource to a receiver slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub source: Key,
    pub resource: Key,
    pub slot: Mark,
    pub permission: Permission,
}

/// Pure module configuration; resource definitions remain owned by providers.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub bindings: &'static [Binding],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigError { MissingSource, BadKey, BadSlot, BadPermission, Collision }

/// Check source-name resolution and distinguish logical keys from receiver Marks.
pub fn validate(config: &Config, sources: &[Key]) -> Result<(), ConfigError> {
    for (at, source) in sources.iter().enumerate() {
        if source.name().is_empty() { return Err(ConfigError::BadKey); }
        if sources[..at].iter().any(|old| old.mark() == source.mark()) {
            return Err(ConfigError::Collision);
        }
    }
    for (at, binding) in config.bindings.iter().enumerate() {
        if binding.source.name().is_empty() || binding.resource.name().is_empty() {
            return Err(ConfigError::BadKey);
        }
        if binding.slot == Mark::NONE { return Err(ConfigError::BadSlot); }
        if binding.permission.is_empty() { return Err(ConfigError::BadPermission); }
        if !sources.iter().any(|source| source.mark() == binding.source.mark() && source.name() == binding.source.name()) {
            return Err(ConfigError::MissingSource);
        }
        if config.bindings[..at].iter().any(|old| {
            old.source.mark() == binding.source.mark()
                && old.resource.mark() == binding.resource.mark()
                && old.resource.name() != binding.resource.name()
        }) { return Err(ConfigError::Collision); }
        if config.bindings[..at].iter().any(|old| {
            old.source.mark() == binding.source.mark() && old.resource.mark() == binding.resource.mark()
                && old.slot == binding.slot
        }) { return Err(ConfigError::Collision); }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Unseen,
    Acquiring,
    Granted,
    Releasing,
    Released,
    Closed,
    Bad,
}

impl State {
    pub const fn code(self) -> u8 {
        match self {
            Self::Unseen => 0,
            Self::Acquiring => 1,
            Self::Granted => 2,
            Self::Releasing => 3,
            Self::Released => 4,
            Self::Closed => 5,
            Self::Bad => 6,
        }
    }

    pub const fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Unseen,
            1 => Self::Acquiring,
            2 => Self::Granted,
            3 => Self::Releasing,
            4 => Self::Released,
            5 => Self::Closed,
            _ => Self::Bad,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    Unknown,
    Bad,
    Denied,
    Conflict,
    Full,
    Pending,
    SourceLost,
}

impl Fail {
    pub const fn code(self) -> u8 {
        match self {
            Self::Unknown => 1,
            Self::Bad => 2,
            Self::Denied => 3,
            Self::Conflict => 4,
            Self::Full => 5,
            Self::Pending => 6,
            Self::SourceLost => 7,
        }
    }

    pub const fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Unknown,
            2 => Self::Bad,
            3 => Self::Denied,
            4 => Self::Conflict,
            5 => Self::Full,
            6 => Self::Pending,
            7 => Self::SourceLost,
            _ => Self::Bad,
        }
    }
}

pub const fn fail_from_code(code: u8) -> Fail {
    Fail::from_code(code)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Delivery {
    /// A token in the receiver's local pie table, returned by the provider's direct ship.
    pub seed: PieToken,
    pub len: u16,
    pub description: [u8; MAX_DESCRIPTION],
}

impl Delivery {
    pub const EMPTY: Self = Self {
        seed: PieToken::NONE,
        len: 0,
        description: [0; MAX_DESCRIPTION],
    };

    pub fn with_description(seed: PieToken, description: &[u8]) -> Option<Self> {
        let len = u16::try_from(description.len()).ok()?;
        if description.len() > MAX_DESCRIPTION {
            return None;
        }
        let mut result = Self::EMPTY;
        result.seed = seed;
        result.len = len;
        result.description[..description.len()].copy_from_slice(description);
        Some(result)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Reply {
    pub status: u8,
    pub state: State,
    pub delivery: Delivery,
}

impl Reply {
    pub fn result(result: Result<(State, Delivery), Fail>) -> Self {
        match result {
            Ok((state, delivery)) => Self { status: wire::OK, state, delivery },
            Err(fail) => Self {
                status: fail.code(),
                state: State::Bad,
                delivery: Delivery::EMPTY,
            },
        }
    }
}

#[mold::interface(id = "sqware.system.supply.v1", metadata)]
mod interface {
    #[channels]
    pub enum Channel {
        #[channel(key = "ask", legacy = "supply-ask", constant = ASK)]
        Ask,
        #[channel(key = "back", legacy = "supply-back", constant = BACK)]
        Back,
    }
    #[grants]
    pub enum Grant {
        #[grant(code = 1, key = "lease", legacy = "supply-entry-lease")]
        Lease,
    }
}

pub use interface::{INTERFACE_ID, REGISTRY};
pub const NAME: &str = "supply";
pub const ASK: Mark = interface::ASK;
pub const BACK: Mark = interface::BACK;

#[derive(env::Frame)]
#[frame(len = 1024)]
pub struct Request {
    pub op: u8,
    pub lease: u64,
    pub target: TaskId,
    pub source: u64,
    pub resource: u64,
    pub slot: u64,
    pub permission: u32,
    pub back: PieToken,
}

impl wire::Message for Request {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];
    fn store(&self, out: &mut [u8]) -> Option<usize> { self.store_at(out, 0) }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        let (request, end) = Self::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(request)
    }
}

#[derive(env::Frame)]
#[frame(len = 512)]
pub struct InspectRequest {
    pub op: u8,
    pub lease: u64,
    pub back: PieToken,
}

impl wire::Message for InspectRequest {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];
    fn store(&self, out: &mut [u8]) -> Option<usize> { self.store_at(out, 0) }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        let (request, end) = Self::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(request)
    }
}

#[derive(env::Frame)]
#[frame(len = 512)]
pub struct CancelRequest {
    pub op: u8,
    pub lease: u64,
    pub back: PieToken,
}

impl wire::Message for CancelRequest {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];
    fn store(&self, out: &mut [u8]) -> Option<usize> { self.store_at(out, 0) }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        let (request, end) = Self::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(request)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Response {
    pub status: u8,
    pub state: u8,
    pub seed: PieToken,
    pub len: u16,
    pub description: [u8; MAX_DESCRIPTION],
}

impl wire::Message for Response {
    type In = Self;
    type Buf = [u8; 140];
    const EMPTY: Self::Buf = [0; 140];
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        let out = out.get_mut(..140)?;
        out[0] = self.status;
        out[1] = self.state;
        out[2..10].copy_from_slice(&self.seed.to_bytes());
        out[10..12].copy_from_slice(&self.len.to_le_bytes());
        let len = self.len as usize;
        if len > MAX_DESCRIPTION { return None; }
        out[12..12 + len].copy_from_slice(&self.description[..len]);
        out[12 + len..140].fill(0);
        Some(140)
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 140 { return None; }
        let len = u16::from_le_bytes(bytes[10..12].try_into().ok()?);
        let count = len as usize;
        if count > MAX_DESCRIPTION { return None; }
        let mut description = [0; MAX_DESCRIPTION];
        description[..count].copy_from_slice(&bytes[12..12 + count]);
        Some(Self {
            status: bytes[0],
            state: bytes[1],
            seed: PieToken::from_bytes(&bytes[2..10])?,
            len,
            description,
        })
    }
}

pub const ACQUIRE: u8 = 1;
pub const INSPECT: u8 = 2;
pub const RELEASE: u8 = 3;
pub const CANCEL: u8 = 4;

fn reply_to(request: &Request) -> PieToken {
    request.back
}

#[mold::contract(request = Request, response = Response, mark = BACK, back = reply_to)]
pub struct Call;
