use env::{Reason, TaskId};

use crate::machine::sync::SpinLock;

mod entry;
pub use entry::Entry;

const RING: usize = 8;

struct Ring {
    entries: [Entry; RING],
    written: usize,
}

impl Ring {
    const fn new() -> Self {
        Self {
            entries: [Entry::EMPTY; RING],
            written: 0,
        }
    }

    fn push(&mut self, entry: Entry) {
        let at = self.written % RING;
        self.entries[at] = entry;
        self.written += 1;
    }

    fn each(&self, mut f: impl FnMut(&Entry)) {
        let start = self.written.saturating_sub(RING);
        for i in start..self.written {
            f(&self.entries[i % RING]);
        }
    }
}

static LEDGER: SpinLock<Ring> = SpinLock::new(Ring::new());

pub fn note(task: TaskId, reason: Reason, note: &str, owner: Option<TaskId>) {
    if reason == 0 || owner.is_some() {
        return;
    }
    let entry = Entry::new(task, reason, note);
    LEDGER.lock().push(entry);
}

pub fn each(f: impl FnMut(&Entry)) {
    LEDGER.lock().each(f);
}
