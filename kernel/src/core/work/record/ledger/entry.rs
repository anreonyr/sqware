use env::{NOTE_MAX, Reason, TaskId};

#[derive(Clone, Copy)]
pub struct Entry {
    pub task: TaskId,
    pub reason: Reason,
    bytes: [u8; NOTE_MAX],
    len: usize,
}

impl Entry {
    pub(super) const EMPTY: Self = Self {
        task: TaskId::new(0),
        reason: 0,
        bytes: [0; NOTE_MAX],
        len: 0,
    };

    pub(super) fn new(task: TaskId, reason: Reason, note: &str) -> Self {
        let raw = note.as_bytes();
        let len = raw.len().min(NOTE_MAX);
        let mut entry = Self {
            task,
            reason,
            bytes: [0; NOTE_MAX],
            len,
        };
        entry.bytes[..len].copy_from_slice(&raw[..len]);
        entry
    }

    pub fn note(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("<non-utf8 note>")
    }
}
