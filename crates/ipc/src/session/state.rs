// Shared lifecycle for aliases of one native session.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU8, Ordering};

use env::{pie, PieToken, TaskId};
use ::resource::raw::{self, Hole};


const READY: u8 = 0;
const BUSY: u8 = 1;
const CLOSED: u8 = 2;

pub(super) struct State(AtomicU8);

impl State {
    pub(super) const fn ready() -> Self {
        Self(AtomicU8::new(READY))
    }
}

pub(super) enum GateFail {
    Busy,
    Closed,
}

/// Validate the capability that a failed exchange may seal.
pub(super) fn valid_reply(reply: PieToken, host: TaskId) -> bool {
    if host == TaskId::new(0) || !raw::alive(reply) {
        return false;
    }
    matches!(raw::reserve(reply), Ok((_, owner, _)) if owner == env::unit::self_id())
}

pub(super) struct CallGuard {
    state: Arc<State>,
    reply: PieToken,
    sent: bool,
    done: bool,
}

pub(super) fn begin(
    state: &Arc<State>,
    reply: PieToken,
    talk: PieToken,
) -> Result<CallGuard, GateFail> {
    match state.0.compare_exchange(READY, BUSY, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => {}
        Err(BUSY) => return Err(GateFail::Busy),
        Err(_) => return Err(GateFail::Closed),
    }

    if !raw::alive(reply) || !raw::alive(talk) {
        state.0.store(CLOSED, Ordering::Release);
        let _ = pie::seal(reply);
        return Err(GateFail::Closed);
    }
    match Hole::from_raw(reply).depth() {
        Ok(0) => {}
        _ => {
            state.0.store(CLOSED, Ordering::Release);
            let _ = pie::seal(reply);
            return Err(GateFail::Closed);
        }
    }

    Ok(CallGuard {
        state: Arc::clone(state),
        reply,
        sent: false,
        done: false,
    })
}

impl CallGuard {
    pub(super) fn sent(&mut self) {
        self.sent = true;
    }

    pub(super) fn complete(&mut self) {
        self.state.0.store(READY, Ordering::Release);
        self.done = true;
    }

    pub(super) fn close(&mut self) {
        self.state.0.store(CLOSED, Ordering::Release);
        let _ = pie::seal(self.reply);
        self.done = true;
    }
}

impl Drop for CallGuard {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        if self.sent {
            self.close();
        } else {
            self.state.0.store(READY, Ordering::Release);
        }
    }
}

pub(super) fn new_state() -> Arc<State> {
    Arc::new(State::ready())
}
