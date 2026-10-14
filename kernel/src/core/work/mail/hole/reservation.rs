use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::machine::clock;

use env::{MailCondition, MailFail, TaskId};

use super::hand::Hand;
use super::pending::Pending;
use super::slot::Slot;
use super::{HANDS_LIVE, HoleMeta, key};
use crate::core::work::room::messenger;

pub(crate) struct Reservation<'a> {
    meta: &'a HoleMeta,
    id: Option<usize>,
}

impl Reservation<'_> {
    pub(crate) fn commit(mut self, buf: Arc<Vec<u8>>, from: TaskId) -> Result<(), MailFail> {
        if buf.is_empty() {
            return Err(MailFail::Denied);
        }
        let at = clock::uptime_ticks();
        {
            let mut pending = self.meta.pending.lock();
            let Pending::Queue(q) = &mut *pending else {
                return Err(MailFail::Dead);
            };
            let at_slot = q
                .hands
                .iter()
                .position(|slot| matches!(slot, Slot::Reserved(id, _) if Some(*id) == self.id))
                .ok_or(MailFail::Dead)?;
            let reserved_len = match q.hands[at_slot] {
                Slot::Reserved(_, len) => len,
                _ => unreachable!(),
            };
            if buf.len() > self.meta.limits.max_len {
                return Err(MailFail::Denied);
            }
            if reserved_len != 0 && reserved_len != buf.len() {
                return Err(MailFail::Denied);
            }
            let next_bytes = q.bytes - reserved_len;
            if buf.len() > self.meta.limits.max_bytes.saturating_sub(next_bytes) {
                return Err(MailFail::Busy);
            }
            q.bytes = next_bytes + buf.len();
            q.hands[at_slot] = Slot::Ready(Hand { from, buf, at });
            HANDS_LIVE.fetch_add(1, Ordering::Relaxed);
            self.id = None;
        }
        let _ = messenger::wake(key(self.meta, MailCondition::Pull), &self.meta.life());
        Ok(())
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        let Some(id) = self.id.take() else {
            return;
        };
        let removed = {
            let mut pending = self.meta.pending.lock();
            match &mut *pending {
                Pending::Queue(q) => {
                    let at = q
                        .hands
                        .iter()
                        .position(|slot| matches!(slot, Slot::Reserved(known, _) if *known == id));
                    if let Some(Slot::Reserved(_, len)) = at.and_then(|at| q.hands.remove(at)) {
                        q.bytes -= len;
                        true
                    } else {
                        false
                    }
                }
                _ => false,
            }
        };
        if removed {
            let _ = messenger::wake(key(self.meta, MailCondition::Pull), &self.meta.life());
            let _ = messenger::wake(key(self.meta, MailCondition::Empty), &self.meta.life());
            let _ = messenger::wake(key(self.meta, MailCondition::Push), &self.meta.life());
        }
    }
}

pub(crate) fn reserve_len(meta: &HoleMeta, len: usize) -> Result<Reservation<'_>, MailFail> {
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    let mut pending = meta.pending.lock();
    let q = match &mut *pending {
        Pending::Queue(q) => q,
        Pending::Dead => return Err(MailFail::Dead),
        Pending::Rung { .. } => return Err(MailFail::Busy),
    };
    if len > meta.limits.max_len {
        return Err(MailFail::Denied);
    }
    if q.hands.len() >= meta.limits.max_messages
        || len > meta.limits.max_bytes.saturating_sub(q.bytes)
    {
        return Err(MailFail::Busy);
    }
    q.hands.try_reserve(1).map_err(|_| MailFail::OoM)?;
    let id = NEXT
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .map_err(|_| MailFail::OoM)?;
    q.hands.push_back(Slot::Reserved(id, len));
    q.bytes += len;
    Ok(Reservation { meta, id: Some(id) })
}
