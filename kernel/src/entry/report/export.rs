use ::core::sync::atomic::{AtomicUsize, Ordering};

use semihosting::io::Write as _;

use crate::core::work::record::trace::{EventKind, HaltEvent};
use crate::entry::report::report::Report;
use crate::machine::sync::SpinLock;

#[path = "export/failure.rs"]
mod failure;

use failure::Failure;

#[derive(serde::Serialize)]
struct HostEvent {
    h: usize,
    when: u64,
    kind: EventKind,
}

pub(crate) fn note_sink(kind: EventKind, hart: usize, when: u64) {
    let Ok(json) = serde_json::to_vec(&HostEvent {
        h: hart,
        when,
        kind,
    }) else {
        fail(Failure::Encode);
        return;
    };
    let wait_ticks = matches!(kind, EventKind::Halt(HaltEvent::Panic)).then_some(HOST_WAIT_TICKS);
    push(&json, wait_ticks);
}

const EXPORT_NAME: &::core::ffi::CStr = c"sqware-diagnose.jsonl";

static FAILURE: AtomicUsize = AtomicUsize::new(0);
static HOST: SpinLock<Option<semihosting::fs::File>> = SpinLock::new(None);

fn fail(failure: Failure) {
    let _ = FAILURE.compare_exchange(0, failure as usize, Ordering::AcqRel, Ordering::Acquire);
}

const HOST_WAIT_TICKS: u64 = 10_000;

fn with_host(wait_ticks: Option<u64>, f: impl FnOnce(&mut semihosting::fs::File) -> bool) {
    let budget = wait_ticks.map(|ticks| (crate::machine::clock::now().as_ticks(), ticks));
    let mut g = loop {
        if FAILURE.load(Ordering::Acquire) != 0 {
            return;
        }
        if let Some(g) = HOST.try_lock() {
            break g;
        }
        match budget {
            // Panic may have stopped the hart holding HOST; ordinary tracing must not discard contention.
            Some((start, ticks)) => {
                if crate::machine::clock::now().as_ticks().wrapping_sub(start) >= ticks {
                    fail(Failure::LockTimeout);
                    return;
                }
            }
            None => crate::machine::stop::hush(),
        }
        ::core::hint::spin_loop();
    };
    if FAILURE.load(Ordering::Acquire) != 0 {
        return;
    }
    if g.is_none() {
        match semihosting::fs::File::create(EXPORT_NAME) {
            Ok(f) => *g = Some(f),
            Err(_) => {
                fail(Failure::Create);
                drop(g);
                crate::putln!(
                    "semihosting: cannot create {:?}; host export disabled",
                    EXPORT_NAME
                );
                return;
            }
        }
    }
    let file = g.as_mut().expect("just stored above");
    if !f(file) {
        fail(Failure::Write);
    }
}

fn push(json: &[u8], wait_ticks: Option<u64>) {
    with_host(wait_ticks, |f| f.write_all(json).and_then(|()| f.write_all(b"\n")).is_ok());
}

pub fn export(r: &Report) {
    let Ok(json) = serde_json::to_vec(r) else {
        fail(Failure::Encode);
        return;
    };
    push(&json, Some(HOST_WAIT_TICKS));
}

/// 仅在停机屏障后读取；失败标记不会在本轮清除。
pub(crate) fn complete() -> bool {
    let failure = FAILURE.load(Ordering::Acquire);
    if let Some(failure) = Failure::decode(failure) {
        crate::putln!("semihosting: structured export failed: {failure:?}");
    }
    failure == 0
}
