use env::{NOTE_MAX, Reason, TaskId};

use crate::lock::SpinLock;

const RING: usize = 8;

#[derive(Clone, Copy)]
pub struct Entry {
    pub task: TaskId,
    pub reason: Reason,
    pub observed: bool,
    bytes: [u8; NOTE_MAX],
    len: usize,
}

impl Entry {
    const EMPTY: Self = Self {
        task: TaskId::new(0),
        reason: 0,
        observed: false,
        bytes: [0; NOTE_MAX],
        len: 0,
    };

    pub fn note(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("<non-utf8 note>")
    }
}

struct Ring {
    entries: [Entry; RING],
    written: usize,
}

static LEDGER: SpinLock<Ring> = SpinLock::new(Ring {
    entries: [Entry::EMPTY; RING],
    written: 0,
});

pub fn note(task: TaskId, reason: Reason, note: &str, observed: bool) {
    if reason == 0 {
        return;
    }
    let raw = note.as_bytes();
    let len = raw.len().min(NOTE_MAX);
    let mut entry = Entry {
        task,
        reason,
        observed,
        bytes: [0; NOTE_MAX],
        len,
    };
    entry.bytes[..len].copy_from_slice(&raw[..len]);

    let mut ring = LEDGER.lock();
    let at = ring.written % RING;
    ring.entries[at] = entry;
    ring.written += 1;
    drop(ring);
    if !observed {
        let mut ring = FAILURES.lock(); let at = ring.written % RING;
        ring.entries[at] = entry; ring.written += 1;
    }
}

pub fn each(f: impl FnMut(&Entry)) {
    let mut f = f;
    let ring = LEDGER.lock();
    let start = ring.written.saturating_sub(RING);
    for i in start..ring.written {
        f(&ring.entries[i % RING]);
    }
}

static FAILURES: SpinLock<Ring> = SpinLock::new(Ring { entries: [Entry::EMPTY; RING], written: 0 });
pub fn failures(mut f: impl FnMut(&Entry)) {
    let ring = FAILURES.lock();
    for i in ring.written.saturating_sub(RING)..ring.written { f(&ring.entries[i % RING]); }
}
