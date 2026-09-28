use core::sync::atomic::{AtomicBool, Ordering};

use semihosting::io::Write as _;

use crate::lock::SpinLock;
use crate::runtime::diagnose::report::Report;

const EXPORT_NAME: &core::ffi::CStr = c"sqware-diagnose.jsonl";

static BROKEN: AtomicBool = AtomicBool::new(false);

const HOST_WAIT_TICKS: u64 = 10_000;

fn with_host(f: impl FnOnce(&mut semihosting::fs::File)) {
    static HOST: SpinLock<Option<semihosting::fs::File>> = SpinLock::new(None);
    let deadline = crate::runtime::chrono::clock::now()
        .as_ticks()
        .wrapping_add(HOST_WAIT_TICKS);
    let mut g = loop {
        if let Some(g) = HOST.try_lock() {
            break g;
        }
        if crate::runtime::chrono::clock::now().as_ticks() >= deadline {
            return;
        }
        core::hint::spin_loop();
    };
    if BROKEN.load(Ordering::Relaxed) {
        return;
    }
    if g.is_none() {
        match semihosting::fs::File::create(EXPORT_NAME) {
            Ok(f) => *g = Some(f),
            Err(_) => {
                BROKEN.store(true, Ordering::Relaxed);
                crate::putln!(
                    "semihosting: cannot create {:?}; host export disabled",
                    EXPORT_NAME
                );
                return;
            }
        }
    }
    let file = g.as_mut().expect("just stored above");
    f(file);
}

pub fn push(json: &[u8]) {
    with_host(|f| {
        let _ = f.write_all(json);
        let _ = f.write_all(b"\n");
    });
}

pub fn export(r: &Report) {
    let json = serde_json::to_vec(r).unwrap_or_default();
    push(&json);
}