use super::kind::Kind;
use super::mark_cell::mark_slot;

#[must_use = "标注靠守卫的生存期起作用：立刻丢掉等于没标"]
pub(crate) fn mark(kind: Kind) -> Mark {
    Mark {
        prev: mark_slot().replace(kind),
    }
}

pub(super) fn current() -> Kind {
    mark_slot().get()
}

pub(crate) struct Mark {
    prev: Kind,
}

impl Drop for Mark {
    fn drop(&mut self) {
        mark_slot().replace(self.prev);
    }
}
