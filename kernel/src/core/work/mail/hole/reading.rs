use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::Ordering;

use env::{HoleCondition, HoleFail, TaskId};

use super::pending::Pending;
use super::slot::Slot;
use super::{HANDS_LIVE, HoleMeta, key, note_hand_off};
use crate::core::work::room::messenger;

pub(crate) struct Reading<'a> {
    meta: &'a HoleMeta,
    pub(crate) from: TaskId,
    pub(crate) bytes: Arc<Vec<u8>>,
    finished: bool,
}

impl Reading<'_> {
    pub(crate) fn finish(mut self) {
        taken(self.meta);
        self.finished = true;
    }
}

impl Drop for Reading<'_> {
    fn drop(&mut self) {
        if !self.finished {
            back(self.meta);
        }
    }
}

pub(crate) fn read(meta: &HoleMeta) -> Result<Reading<'_>, HoleFail> {
    let mut pending = meta.pending.lock();
    let queue = match &mut *pending {
        Pending::Queue(queue) => queue,
        Pending::Dead => return Err(HoleFail::Dead),
        _ => return Err(HoleFail::Busy),
    };
    if queue.taking {
        return Err(HoleFail::Busy);
    }
    let head = queue.head().ok_or(HoleFail::Busy)?;
    let reading = Reading {
        meta,
        from: head.from,
        bytes: head.buf.clone(),
        finished: false,
    };
    queue.taking = true;
    Ok(reading)
}

pub(crate) fn peek(meta: &HoleMeta) -> Result<(usize, TaskId, usize), HoleFail> {
    if !meta.alive() {
        return Err(HoleFail::Dead);
    }
    let pending = meta.pending.lock();
    match &*pending {
        Pending::Queue(q) => match q.head() {
            Some(hand) => Ok((
                hand.buf.len(),
                hand.from,
                q.hands.iter().filter_map(Slot::hand).count(),
            )),
            None => Err(HoleFail::Busy),
        },
        _ => Err(HoleFail::Busy),
    }
}

pub(super) fn taken(meta: &HoleMeta) {
    let mut off = None;
    {
        let mut pending = meta.pending.lock();
        if let Pending::Queue(q) = &mut *pending {
            if q.taking {
                q.taking = false;
                off = q.hands.pop_front().and_then(|slot| match slot {
                    Slot::Ready(hand) => {
                        q.bytes -= hand.buf.len();
                        Some(hand)
                    }
                    Slot::Reserved(_, len) => {
                        q.bytes -= len;
                        None
                    }
                });
            }
        }
    }
    if let Some(hand) = off {
        let len = hand.buf.len();
        HANDS_LIVE.fetch_sub(1, Ordering::Relaxed);
        note_hand_off(meta, hand.from, len, hand.at);
    }
    let _ = messenger::wake(key(meta, HoleCondition::Empty), &meta.life());
    let _ = messenger::wake(key(meta, HoleCondition::Push), &meta.life());
    let _ = messenger::wake(key(meta, HoleCondition::Pull), &meta.life());
}

pub(super) fn back(meta: &HoleMeta) {
    {
        let mut pending = meta.pending.lock();
        if let Pending::Queue(q) = &mut *pending {
            q.taking = false;
        }
    }
    let _ = messenger::wake(key(meta, HoleCondition::Pull), &meta.life());
}
