use alloc::vec::Vec;
use env::{Access, Mark, PieToken, TaskId};

use super::{EntryId, Permit, path::PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry {
    pub operator: TaskId,
    pub target: EntryTarget,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EntryTarget {
    Root,
    Node(EntryId),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Binding {
    pub entry: Entry,
    pub revision: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BoundResource {
    pub binding: Binding,
    pub resource: PieToken,
    pub view: ResourceView,
    pub permit: Permit,
}

/// The exact consumer operations exposed through a bound resource reference.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ResourceView(Access);

impl ResourceView {
    pub fn new(access: Access) -> Option<Self> {
        (access != Access::NONE).then_some(Self(access))
    }
    pub const fn access(self) -> Access {
        self.0
    }
    pub const fn permission(self) -> env::Permission {
        self.0.bits()
    }
    pub const fn bits(self) -> u32 {
        self.0.bits().bits()
    }
    pub fn fits(self, permission: env::Permission) -> bool {
        permission.contains(self.0.bits() | env::Permission::VEST)
    }
    pub const fn from_bits(bits: u32) -> Option<Self> {
        match Access::from_bits(bits) {
            Some(access) if access.bits().bits() != 0 => Some(Self(access)),
            _ => None,
        }
    }
}

impl Binding {
    pub const fn entry_id(self) -> EntryId {
        match self.entry.target {
            EntryTarget::Node(id) => id,
            EntryTarget::Root => EntryId::new(0),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GrantId(pub u64);

/// Caller-assigned, monotonically increasing identifier for a PART/BIND mutation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OperationId(pub u64);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OperationState {
    Unseen,
    Pending,
    Applied,
    Closed,
    Failed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ops(u16);

impl Ops {
    pub const NONE: Self = Self(0);
    pub const PART: Self = Self(1 << 0);
    pub const UNPART: Self = Self(1 << 1);
    pub const BIND: Self = Self(1 << 2);
    pub const REPLACE: Self = Self(1 << 3);
    pub const UNBIND: Self = Self(1 << 4);
    pub const INSPECT: Self = Self(1 << 5);
    pub const GRANT: Self = Self(1 << 6);
    pub const REVOKE: Self = Self(1 << 7);
    pub const FIND: Self = Self(1 << 8);
    pub const CANCEL: Self = Self(1 << 9);
    pub const ALL: Self = Self((1 << 10) - 1);

    pub const fn bits(self) -> u16 {
        self.0
    }
    pub const fn from_bits(bits: u64) -> Option<Self> {
        if bits <= u16::MAX as u64 {
            Some(Self(bits as u16))
        } else {
            None
        }
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    pub const fn is_valid(self) -> bool {
        self.0 & !Self::ALL.0 == 0
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Scope {
    Subtree,
    Names(Vec<PathBuf>),
}

impl Scope {
    pub fn allows(&self, relative: &str) -> bool {
        match self {
            Self::Subtree => true,
            Self::Names(names) => names.iter().any(|name| {
                let prefix = name.as_str();
                relative == prefix
                    || relative
                        .strip_prefix(prefix)
                        .is_some_and(|tail| tail.starts_with('/'))
            }),
        }
    }

    pub fn narrows(&self, child: &Self) -> bool {
        match (self, child) {
            (Self::Subtree, _) => true,
            (Self::Names(parent), Self::Names(child)) => child.iter().all(|name| {
                parent.iter().any(|prefix| {
                    let prefix = prefix.as_str();
                    let name = name.as_str();
                    name == prefix
                        || name
                            .strip_prefix(prefix)
                            .is_some_and(|tail| tail.starts_with('/'))
                })
            }),
            (Self::Names(_), Self::Subtree) => false,
        }
    }

    /// Whether a child scope, relative to its own anchor, remains inside this scope.
    pub fn narrows_at(&self, anchor: &str, child: &Self) -> bool {
        match self {
            Self::Subtree => true,
            Self::Names(names) => {
                if names.iter().any(|name| {
                    let prefix = name.as_str();
                    anchor == prefix
                        || anchor
                            .strip_prefix(prefix)
                            .is_some_and(|tail| tail.starts_with('/'))
                }) {
                    return true;
                }
                match child {
                    Self::Subtree => false,
                    Self::Names(children) => children.iter().all(|child| {
                        names.iter().any(|name| {
                            let Some(tail) = name
                                .as_str()
                                .strip_prefix(anchor)
                                .and_then(|tail| tail.strip_prefix('/'))
                            else {
                                return false;
                            };
                            let path = child.as_str();
                            path == tail
                                || path
                                    .strip_prefix(tail)
                                    .is_some_and(|rest| rest.starts_with('/'))
                        })
                    }),
                }
            }
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GrantSpec {
    pub id: GrantId,
    pub holder: TaskId,
    pub anchor: Entry,
    pub scope: Scope,
    pub ops: Ops,
    pub permit: Option<Permit>,
    pub source: Option<TaskId>,
    pub source_mark: Option<Mark>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GrantState {
    Unseen,
    Pending,
    Granted,
    Revoking,
    Closed,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GrantSubmission {
    pub caller: TaskId,
    pub via: PieToken,
    pub parent: Option<GrantId>,
    pub spec: GrantSpec,
}

/// Fixed native descriptor for one unit-local namespace authority.
#[derive(Clone, Copy, PartialEq, Eq, Debug, env::Frame)]
pub struct GrantDescriptor {
    pub id: u64,
    pub entry_operator: TaskId,
    pub entry_root: u8,
    pub entry_id: super::EntryId,
    pub entrance: PieToken,
}

/// Per-namespace mark used to inject the one descriptor for that declaration index.
pub const fn descriptor_mark(index: usize) -> Option<Mark> {
    if index >= 32 {
        None
    } else {
        Some(Mark::new(0x5359_534f_0000_0000 | index as u64))
    }
}
