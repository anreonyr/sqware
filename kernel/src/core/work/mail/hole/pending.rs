use alloc::collections::VecDeque;

use env::{MailCondition, MailFail};

use super::queue::Queue;
use super::slot::Slot;
use super::{HoleMeta, key};
use crate::core::work::room::messenger;

pub(super) enum Pending {
    Queue(Queue),
    Rung { count: usize, spare: VecDeque<Slot> },
    Dead,
}

pub(super) const RING_CAP: usize = 4;

pub(crate) fn ring(meta: &HoleMeta) -> Result<(), MailFail> {
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    {
        let mut pending = meta.pending.lock();
        match &mut *pending {
            Pending::Queue(q) if q.hands.is_empty() && !q.taking => {
                *pending = Pending::Rung {
                    count: 1,
                    spare: core::mem::take(&mut q.hands),
                };
            }
            Pending::Rung { count, .. } if *count < RING_CAP => {
                *count += 1;
            }
            _ => return Err(MailFail::Busy),
        }
    };
    let _ = messenger::wake(key(meta, MailCondition::Pull), &meta.life());
    Ok(())
}

pub(crate) fn hush(meta: &HoleMeta) -> Result<(), MailFail> {
    let mut pending = meta.pending.lock();
    match &mut *pending {
        Pending::Rung { count, .. } if *count > 1 => *count -= 1,
        Pending::Rung { spare, .. } => {
            *pending = Pending::Queue(Queue {
                hands: core::mem::take(spare),
                taking: false,
                bytes: 0,
            });
        }
        _ => return Err(MailFail::Busy),
    }
    drop(pending);
    let _ = messenger::wake(key(meta, MailCondition::Push), &meta.life());
    let _ = messenger::wake(key(meta, MailCondition::Empty), &meta.life());
    Ok(())
}
