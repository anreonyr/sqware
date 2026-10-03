use super::super::limits::{MAX_ACTIVE_COALITIONS, MAX_PAGE_ITEMS};
use env::TaskId;
use env::wire::Span;

pub use crate::wire::OK;

#[derive(Clone, Copy, Debug, PartialEq, Eq, env::WireCodes)]
pub enum Fail {
    #[code(1)]
    Bad,
    #[code(2)]
    UnknownPrincipal,
    #[code(3)]
    UnknownCoalition,
    #[code(4)]
    WrongAuthority,
    #[code(5)]
    Denied,
    #[code(6)]
    NotManager,
    #[code(7)]
    NotNarrower,
    #[code(8)]
    NotEligible,
    #[code(9)]
    Full,
    #[code(10)]
    Changed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wire {
    Resolve(TaskId),
    Matches(TaskId, Selector),
    Same(TaskId, TaskId),
    Sire(PrincipalId),
    Heir(PrincipalId, PrincipalId),
    Amid(PrincipalId, CoalitionId),
    Members(CoalitionId, Option<Cursor>),
    Memberships(PrincipalId, Option<Cursor>),
    Adopt(Subject),
    Waive,
    Restrict(Subject),
    Derive(PrincipalId),
    Found,
    Admit(CoalitionId, PrincipalId),
    Expel(CoalitionId, PrincipalId),
    Bind(TaskId, Install),
    Unbind(TaskId),
}
impl Wire {
    pub(super) fn validate(self) -> Option<()> {
        match self {
            Self::Heir(a, b) => (a.authority == b.authority).then_some(()),
            Self::Amid(p, c) | Self::Admit(c, p) | Self::Expel(c, p) => {
                (p.authority == c.authority).then_some(())
            }
            Self::Members(c, k) => {
                (!k.is_some_and(|k| k.target != PageTarget::Members(c))).then_some(())
            }
            Self::Memberships(p, k) => {
                (!k.is_some_and(|k| k.target != PageTarget::Memberships(p))).then_some(())
            }
            _ => Some(()),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    Unit,
    Binding(Option<Binding>),
    Principal(Option<PrincipalId>),
    Coalition(CoalitionId),
    Match(Match),
    Bool(bool),
    Members(Page<PrincipalId>),
    Memberships(Page<CoalitionId>),
    Fail(Fail),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, env::Frame)]
pub struct PrincipalId {
    pub authority: TaskId,
    pub slot: u64,
}
impl PrincipalId {
    pub const fn new(authority: TaskId, slot: u64) -> Self {
        Self { authority, slot }
    }
    pub const fn root(authority: TaskId) -> Self {
        Self { authority, slot: 0 }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, env::Frame)]
pub struct CoalitionId {
    pub authority: TaskId,
    pub slot: u64,
}
impl CoalitionId {
    pub const fn new(authority: TaskId, slot: u64) -> Self {
        Self { authority, slot }
    }
    pub const fn root(authority: TaskId) -> Self {
        Self { authority, slot: 0 }
    }
}

/// Canonically sorted, unique, single-authority, allocation-free active selections.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoalitionSet {
    items: [CoalitionId; MAX_ACTIVE_COALITIONS],
    len: u8,
}
impl CoalitionSet {
    pub const fn empty() -> Self {
        Self {
            items: [CoalitionId {
                authority: TaskId::new(0),
                slot: 0,
            }; MAX_ACTIVE_COALITIONS],
            len: 0,
        }
    }
    pub fn new(values: &[CoalitionId]) -> Result<Self, Fail> {
        if values.len() > MAX_ACTIVE_COALITIONS {
            return Err(Fail::Full);
        }
        let mut set = Self::empty();
        for &value in values {
            if value.authority.get() == 0 {
                return Err(Fail::Bad);
            }
            if values[0].authority != value.authority {
                return Err(Fail::WrongAuthority);
            }
            set.items[set.len as usize] = value;
            set.len += 1;
        }
        set.items[..set.len as usize].sort_unstable();
        if set.as_slice().windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(Fail::Bad);
        }
        Ok(set)
    }
    pub fn as_slice(&self) -> &[CoalitionId] {
        &self.items[..self.len as usize]
    }
    pub fn iter(&self) -> impl Iterator<Item = CoalitionId> + '_ {
        self.as_slice().iter().copied()
    }
    pub fn len(&self) -> usize {
        self.len as usize
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn contains(&self, value: CoalitionId) -> bool {
        self.as_slice().binary_search(&value).is_ok()
    }
    pub fn is_subset(&self, other: &Self) -> bool {
        self.iter().all(|id| other.contains(id))
    }
    pub fn remove(&mut self, value: CoalitionId) {
        self.retain(|id| id != value);
    }
    pub fn retain(&mut self, mut keep: impl FnMut(CoalitionId) -> bool) {
        let mut n = 0;
        for i in 0..self.len as usize {
            if keep(self.items[i]) {
                self.items[n] = self.items[i];
                n += 1;
            }
        }
        self.items[n..].fill(CoalitionId {
            authority: TaskId::new(0),
            slot: 0,
        });
        self.len = n as u8;
    }
}
impl Default for CoalitionSet {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Subject {
    pub principal: PrincipalId,
    pub coalitions: CoalitionSet,
}
impl Subject {
    pub fn new(principal: PrincipalId, coalitions: &[CoalitionId]) -> Result<Self, Fail> {
        let subject = Self {
            principal,
            coalitions: CoalitionSet::new(coalitions)?,
        };
        subject.validate()?;
        Ok(subject)
    }
    pub fn validate(&self) -> Result<(), Fail> {
        if self.principal.authority.get() == 0 {
            return Err(Fail::Bad);
        }
        if self
            .coalitions
            .iter()
            .any(|c| c.authority != self.principal.authority)
        {
            return Err(Fail::WrongAuthority);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub origin: Subject,
    pub current: Subject,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selector {
    Exact(PrincipalId),
    DescendantOf(PrincipalId),
    MemberOf(CoalitionId),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Match {
    Yes,
    No,
    Unbound,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Install {
    Authorized(Subject),
    Inherit { parent: TaskId },
    Restrict { parent: TaskId, subject: Subject },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageTarget {
    Members(CoalitionId),
    Memberships(PrincipalId),
}
impl PageTarget {
    pub fn authority(self) -> TaskId {
        match self {
            Self::Members(c) => c.authority,
            Self::Memberships(p) => p.authority,
        }
    }
}
/// `None` starts a scan. Slot zero is a real threshold, not a sentinel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub target: PageTarget,
    pub revision: u64,
    pub after: u64,
}

/// **一条 id 轴**：每枚 id 自己就是过线的一格（`Span`），故一页的字段表可以泛在一枚 id 上。
pub trait PageId: Span + Copy + Ord + core::fmt::Debug + Eq {
    const EMPTY: Self;
    fn authority(self) -> TaskId;
    fn slot(self) -> u64;
    fn accepts(target: PageTarget) -> bool;
}
impl PageId for PrincipalId {
    const EMPTY: Self = Self {
        authority: TaskId::new(0),
        slot: 0,
    };
    fn authority(self) -> TaskId {
        self.authority
    }
    fn slot(self) -> u64 {
        self.slot
    }
    fn accepts(target: PageTarget) -> bool {
        matches!(target, PageTarget::Members(_))
    }
}
impl PageId for CoalitionId {
    const EMPTY: Self = Self {
        authority: TaskId::new(0),
        slot: 0,
    };
    fn authority(self) -> TaskId {
        self.authority
    }
    fn slot(self) -> u64 {
        self.slot
    }
    fn accepts(target: PageTarget) -> bool {
        matches!(target, PageTarget::Memberships(_))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Page<T: PageId> {
    items: [T; MAX_PAGE_ITEMS],
    len: u8,
    next: Option<Cursor>,
}
impl<T: PageId> Page<T> {
    pub fn new(values: &[T], next: Option<Cursor>) -> Result<Self, Fail> {
        if values.len() > MAX_PAGE_ITEMS {
            return Err(Fail::Full);
        }
        if values.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(Fail::Bad);
        }
        if values.iter().any(|id| id.authority().get() == 0) {
            return Err(Fail::Bad);
        }
        if let Some(first) = values.first() {
            if values.iter().any(|id| id.authority() != first.authority()) {
                return Err(Fail::WrongAuthority);
            }
        }
        if let Some(cursor) = next {
            if !T::accepts(cursor.target) || cursor.target.authority().get() == 0 {
                return Err(Fail::Bad);
            }
            let last = values.last().ok_or(Fail::Bad)?;
            if last.authority() != cursor.target.authority() {
                return Err(Fail::WrongAuthority);
            }
            if last.slot() != cursor.after {
                return Err(Fail::Bad);
            }
        }
        let mut page = Self {
            items: [T::EMPTY; MAX_PAGE_ITEMS],
            len: values.len() as u8,
            next,
        };
        page.items[..values.len()].copy_from_slice(values);
        Ok(page)
    }
    pub fn as_slice(&self) -> &[T] {
        &self.items[..self.len as usize]
    }
    pub fn iter(&self) -> impl Iterator<Item = T> + '_ {
        self.as_slice().iter().copied()
    }
    pub fn next(&self) -> Option<Cursor> {
        self.next
    }
    pub fn len(&self) -> usize {
        self.len as usize
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn more(&self) -> bool {
        self.next.is_some()
    }
}
