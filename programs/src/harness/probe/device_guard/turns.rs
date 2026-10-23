use ::schedule::Cursor;

/// The single app-frame cursor carried across one root-frame tick.
#[derive(Default)]
pub(super) struct Turns {
    cursor: Cursor,
}

impl Turns {
    pub(super) fn take(&mut self) -> Cursor {
        core::mem::take(&mut self.cursor)
    }

    pub(super) fn put(&mut self, cursor: Cursor) {
        self.cursor = cursor;
    }
}
