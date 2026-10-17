use ::core::sync::atomic::{AtomicBool, Ordering};

use semihosting::io::Write as _;

use crate::core::work::record::trace::EventKind;
use crate::entry::report::report::Report;
use crate::machine::sync::SpinLock;

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
        BROKEN.store(true, Ordering::Release);
        return;
    };
    push(&json);
}

const EXPORT_NAME: &::core::ffi::CStr = c"sqware-diagnose.jsonl";

static BROKEN: AtomicBool = AtomicBool::new(false);

const HOST_WAIT_TICKS: u64 = 10_000;

fn with_host(f: impl FnOnce(&mut semihosting::fs::File) -> bool) {
    static HOST: SpinLock<Option<semihosting::fs::File>> = SpinLock::new(None);
    let deadline = crate::machine::clock::now()
        .as_ticks()
        .wrapping_add(HOST_WAIT_TICKS);
    let mut g = loop {
        if let Some(g) = HOST.try_lock() {
            break g;
        }
        if crate::machine::clock::now().as_ticks() >= deadline {
            BROKEN.store(true, Ordering::Release);
            return;
        }
        ::core::hint::spin_loop();
    };
    if BROKEN.load(Ordering::Relaxed) {
        return;
    }
    if g.is_none() {
        match semihosting::fs::File::create(EXPORT_NAME) {
            Ok(f) => *g = Some(f),
            Err(_) => {
                BROKEN.store(true, Ordering::Release);
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
        BROKEN.store(true, Ordering::Release);
    }
}

pub fn push(json: &[u8]) {
    with_host(|f| f.write_all(json).and_then(|()| f.write_all(b"\n")).is_ok());
}

pub fn export(r: &Report) {
    let Ok(json) = serde_json::to_vec(r) else {
        BROKEN.store(true, Ordering::Release);
        return;
    };
    push(&json);
}

/// 仅在停机屏障后读取；失败标记不会在本轮清除。
pub(crate) fn complete() -> bool {
    !BROKEN.load(Ordering::Acquire)
}
