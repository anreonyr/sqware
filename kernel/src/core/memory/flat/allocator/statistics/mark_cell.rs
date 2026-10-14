use super::kind::Kind;

pub(super) struct MarkCell(core::cell::Cell<Kind>);

unsafe impl Sync for MarkCell {}

static MARK: [MarkCell; crate::machine::layout::MAX_HART_SLOTS] =
    [const { MarkCell(core::cell::Cell::new(Kind::Plain)) }; crate::machine::layout::MAX_HART_SLOTS];

pub(super) fn mark_slot() -> &'static MarkCell {
    &MARK[crate::machine::hart::hart_id()
        .get()
        .min(crate::machine::layout::MAX_HART_SLOTS - 1)]
}

impl MarkCell {
    pub(super) fn replace(&self, kind: Kind) -> Kind {
        self.0.replace(kind)
    }

    pub(super) fn get(&self) -> Kind {
        self.0.get()
    }
}
