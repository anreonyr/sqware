//! Checked identity fields and counted snapshots; Frame owns their layout.

use super::super::limits::{MAX_ACTIVE_COALITIONS, MAX_PAGE_ITEMS};
use super::vocab::*;
use env::{
    PieToken, TaskId,
    wire::{Field, Span},
};

pub(super) struct Task(pub TaskId);
impl Task {
    pub(super) fn new(task: TaskId) -> Option<Self> {
        (task.get() != 0).then_some(Self(task))
    }
}
impl Field for Task {
    const WIDTH: usize = TaskId::WIDTH;
    fn store(&self, out: &mut [u8]) {
        Field::store(&self.0, out);
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        Self::new(TaskId::new(usize::try_from(u64::fetch(bytes)?).ok()?))
    }
}

pub(super) struct Back(pub PieToken);
impl Field for Back {
    const WIDTH: usize = PieToken::WIDTH;
    fn store(&self, out: &mut [u8]) {
        Field::store(&self.0, out);
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        usize::try_from(u64::fetch(bytes)?).ok()?;
        Some(Self(PieToken::fetch(bytes)?))
    }
}

#[derive(env::Frame)]
struct IdFrame {
    authority: Task,
    slot: u64,
}

#[derive(env::Frame)]
struct SelectorFrame {
    kind: u8,
    id: IdFrame,
}
impl Field for Selector {
    const WIDTH: usize = SelectorFrame::LEN;
    fn store(&self, out: &mut [u8]) {
        let (kind, authority, slot) = match *self {
            Self::Exact(p) => (0, p.authority, p.slot),
            Self::DescendantOf(p) => (1, p.authority, p.slot),
            Self::MemberOf(c) => (2, c.authority, c.slot),
        };
        let _ = SelectorFrame {
            kind,
            id: IdFrame {
                authority: Task(authority),
                slot,
            },
        }
        .store_at(out, 0);
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        let (frame, at) = SelectorFrame::fetch_at(bytes, 0)?;
        if at != bytes.len() {
            return None;
        }
        let IdFrame {
            authority: Task(authority),
            slot,
        } = frame.id;
        match frame.kind {
            0 => Some(Self::Exact(PrincipalId::new(authority, slot))),
            1 => Some(Self::DescendantOf(PrincipalId::new(authority, slot))),
            2 => Some(Self::MemberOf(CoalitionId::new(authority, slot))),
            _ => None,
        }
    }
}
pub(super) fn valid_selector(selector: Selector) -> Option<()> {
    let authority = match selector {
        Selector::Exact(p) | Selector::DescendantOf(p) => p.authority,
        Selector::MemberOf(c) => c.authority,
    };
    Task::new(authority).map(|_| ())
}

#[derive(env::Frame)]
struct SubjectFrame {
    principal: PrincipalId,
    count: u8,
    #[frame(count = count, fill = CoalitionId::EMPTY)]
    coalitions: [CoalitionId; MAX_ACTIVE_COALITIONS],
}
impl Span for Subject {
    const MAX: Option<usize> = Some(SubjectFrame::LEN);
    fn store_at(&self, out: &mut [u8], at: usize) -> Option<usize> {
        self.validate().ok()?;
        let mut coalitions = [CoalitionId::EMPTY; MAX_ACTIVE_COALITIONS];
        coalitions[..self.coalitions.len()].copy_from_slice(self.coalitions.as_slice());
        SubjectFrame {
            principal: self.principal,
            count: self.coalitions.len() as u8,
            coalitions,
        }
        .store_at(out, at)
    }
    fn fetch_at(bytes: &[u8], at: usize) -> Option<(Self, usize)> {
        let (frame, at) = SubjectFrame::fetch_at(bytes, at)?;
        let coalitions = &frame.coalitions[..frame.count as usize];
        if coalitions.windows(2).any(|pair| pair[0] >= pair[1]) {
            return None;
        }
        Some((Self::new(frame.principal, coalitions).ok()?, at))
    }
}

#[derive(env::Frame)]
struct BindingFrame {
    origin: Subject,
    current: Subject,
}
fn valid_binding(binding: Binding) -> Option<()> {
    (binding.origin.principal.authority == binding.current.principal.authority
        && binding
            .current
            .coalitions
            .is_subset(&binding.origin.coalitions))
    .then_some(())
}
impl Span for Binding {
    const MAX: Option<usize> = Some(BindingFrame::LEN);
    fn store_at(&self, out: &mut [u8], at: usize) -> Option<usize> {
        valid_binding(*self)?;
        BindingFrame {
            origin: self.origin,
            current: self.current,
        }
        .store_at(out, at)
    }
    fn fetch_at(bytes: &[u8], at: usize) -> Option<(Self, usize)> {
        let (frame, at) = BindingFrame::fetch_at(bytes, at)?;
        let binding = Self {
            origin: frame.origin,
            current: frame.current,
        };
        valid_binding(binding)?;
        Some((binding, at))
    }
}

#[derive(env::Frame)]
struct CursorFrame {
    kind: u8,
    id: IdFrame,
    revision: u64,
    after: u64,
}
impl Span for Cursor {
    const MAX: Option<usize> = Some(CursorFrame::LEN);
    fn store_at(&self, out: &mut [u8], at: usize) -> Option<usize> {
        let (kind, authority, slot) = match self.target {
            PageTarget::Members(c) => (0, c.authority, c.slot),
            PageTarget::Memberships(p) => (1, p.authority, p.slot),
        };
        CursorFrame {
            kind,
            id: IdFrame {
                authority: Task::new(authority)?,
                slot,
            },
            revision: self.revision,
            after: self.after,
        }
        .store_at(out, at)
    }
    fn fetch_at(bytes: &[u8], at: usize) -> Option<(Self, usize)> {
        let (frame, at) = CursorFrame::fetch_at(bytes, at)?;
        let IdFrame {
            authority: Task(authority),
            slot,
        } = frame.id;
        let target = match frame.kind {
            0 => PageTarget::Members(CoalitionId::new(authority, slot)),
            1 => PageTarget::Memberships(PrincipalId::new(authority, slot)),
            _ => return None,
        };
        Some((
            Self {
                target,
                revision: frame.revision,
                after: frame.after,
            },
            at,
        ))
    }
}

/// The protocol's optional value is a presence flag followed only by a present value.
pub(super) struct Optional<T>(pub Option<T>);
impl<T: Span> Span for Optional<T> {
    const MAX: Option<usize> = env::wire::total(&[<bool as Span>::MAX, T::MAX]);
    fn store_at(&self, out: &mut [u8], at: usize) -> Option<usize> {
        let at = self.0.is_some().store_at(out, at)?;
        match &self.0 {
            Some(value) => value.store_at(out, at),
            None => Some(at),
        }
    }
    fn fetch_at(bytes: &[u8], at: usize) -> Option<(Self, usize)> {
        let (present, at) = bool::fetch_at(bytes, at)?;
        if !present {
            return Some((Self(None), at));
        }
        let (value, at) = T::fetch_at(bytes, at)?;
        Some((Self(Some(value)), at))
    }
}

/// **一页的字段表**：一条轴一份（`PrincipalId` 那一向、`CoalitionId` 那一向同形），故它泛在
/// 那枚 id 上——`ids` 那一格的元素类型就是参数，`fill` 取那一枚 id 自己的空位。
#[derive(env::Frame)]
struct PageFrame<T: PageId> {
    count: u8,
    #[frame(count = count, fill = T::EMPTY)]
    ids: [T; MAX_PAGE_ITEMS],
    next: Optional<Cursor>,
}

impl<T: PageId> Span for Page<T> {
    const MAX: Option<usize> = Some(PageFrame::<T>::LEN);
    fn store_at(&self, out: &mut [u8], at: usize) -> Option<usize> {
        let mut ids = [T::EMPTY; MAX_PAGE_ITEMS];
        ids[..self.len()].copy_from_slice(self.as_slice());
        PageFrame {
            count: self.len() as u8,
            ids,
            next: Optional(self.next()),
        }
        .store_at(out, at)
    }
    fn fetch_at(bytes: &[u8], at: usize) -> Option<(Self, usize)> {
        let (frame, at) = <PageFrame<T> as Span>::fetch_at(bytes, at)?;
        Some((
            Self::new(&frame.ids[..frame.count as usize], frame.next.0).ok()?,
            at,
        ))
    }
}

impl Field for Match {
    const WIDTH: usize = u8::WIDTH;
    fn store(&self, out: &mut [u8]) {
        Field::store(
            &match self {
                Self::Yes => 0u8,
                Self::No => 1,
                Self::Unbound => 2,
            },
            out,
        );
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        match u8::fetch(bytes)? {
            0 => Some(Self::Yes),
            1 => Some(Self::No),
            2 => Some(Self::Unbound),
            _ => None,
        }
    }
}
