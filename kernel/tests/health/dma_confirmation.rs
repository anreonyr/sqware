//! Inject selected RPC or reference-release failures, not hardware acknowledgements.
//! Ownership evidence comes from the live IOMMU task's reference table.

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use env::TaskId;

#[path = "../../../programs/src/harness/probe/device_confirmation/injection.rs"]
#[allow(dead_code)]
mod injection;

static ENABLED: AtomicBool = AtomicBool::new(false);
static FROM: AtomicUsize = AtomicUsize::new(0);
static TO: AtomicUsize = AtomicUsize::new(0);
static OP: AtomicU64 = AtomicU64::new(0);
static SERIAL: AtomicU64 = AtomicU64::new(0);
static RETAINER: AtomicUsize = AtomicUsize::new(0);
static HITS: AtomicUsize = AtomicUsize::new(0);
static REFERENCE: AtomicUsize = AtomicUsize::new(0);

fn word(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

fn retained_references() -> usize {
    let retainer = TaskId::new(RETAINER.load(Ordering::Acquire));
    let task = crate::core::work::room::scheduler::roster::muster(retainer)
        .and_then(|task| task.upgrade())
        .expect("confirmation: live IOMMU");
    task.pend.references.len()
}

pub(crate) fn reject(owner: TaskId, from: TaskId, root: bool, bytes: &[u8]) -> bool {
    if !crate::testing() {
        return false;
    }
    if root && from == owner && bytes.len() == injection::LEN && &bytes[..8] == injection::TAG {
        let enabled = word(bytes, 8) == 1;
        if enabled {
            ENABLED.store(false, Ordering::Release);
            FROM.store(word(bytes, 16) as usize, Ordering::Relaxed);
            TO.store(word(bytes, 24) as usize, Ordering::Relaxed);
            OP.store(word(bytes, 32), Ordering::Relaxed);
            SERIAL.store(word(bytes, 40), Ordering::Relaxed);
            RETAINER.store(word(bytes, 48) as usize, Ordering::Relaxed);
            HITS.store(0, Ordering::Relaxed);
            REFERENCE.store(0, Ordering::Relaxed);
        } else {
            let references = retained_references();
            assert_eq!(
                references, 2,
                "confirmation: table and DMA references remain owned"
            );
            if OP.load(Ordering::Relaxed) == injection::REFERENCE_RELEASE {
                let reference = REFERENCE.load(Ordering::Acquire);
                assert_ne!(reference, 0, "confirmation: Reference Release was reached");
                assert!(
                    reference_owned(reference),
                    "confirmation: failed reference was removed"
                );
                crate::putln!(
                    "confirmation: failed reference still owned reference={reference} pages=6"
                );
            }
            crate::putln!(
                "confirmation: injection removed hits={} references={references} ticks={}",
                HITS.load(Ordering::Acquire),
                crate::machine::clock::now().as_ticks(),
            );
        }
        ENABLED.store(enabled, Ordering::Release);
        return false;
    }
    if !ENABLED.load(Ordering::Acquire)
        || from.get() != FROM.load(Ordering::Relaxed)
        || owner.get() != TO.load(Ordering::Relaxed)
        || bytes.len() < 9
        || u64::from(bytes[0]) != OP.load(Ordering::Relaxed)
        || word(bytes, 1) != SERIAL.load(Ordering::Relaxed)
    {
        return false;
    }
    if HITS.fetch_add(1, Ordering::AcqRel) == 0 {
        let references = retained_references();
        assert_eq!(
            references, 2,
            "confirmation: table and DMA references remain owned"
        );
        crate::putln!(
            "confirmation: transport rejected from={} to={} op={} serial={} references={references} ticks={}",
            from.get(),
            owner.get(),
            bytes[0],
            word(bytes, 1),
            crate::machine::clock::now().as_ticks(),
        );
    }
    true
}

fn reference_owned(reference: usize) -> bool {
    let retainer = TaskId::new(RETAINER.load(Ordering::Acquire));
    let Some(task) = crate::core::work::room::scheduler::roster::muster(retainer)
        .and_then(|task| task.upgrade())
    else {
        return false;
    };
    task.pend
        .references
        .get(reference)
        .is_some_and(|hold| hold.retains() && hold.window().1.get() == 6)
}

pub(crate) fn reject_reference(from: TaskId, reference: usize) -> bool {
    if !crate::testing()
        || !ENABLED.load(Ordering::Acquire)
        || OP.load(Ordering::Relaxed) != injection::REFERENCE_RELEASE
        || from.get() != TO.load(Ordering::Relaxed)
        || from.get() != RETAINER.load(Ordering::Relaxed)
        || !reference_owned(reference)
    {
        return false;
    }
    let previous = REFERENCE
        .compare_exchange(0, reference, Ordering::AcqRel, Ordering::Acquire)
        .unwrap_or_else(|previous| previous);
    assert!(
        previous == 0 || previous == reference,
        "confirmation: retry changed reference handle"
    );
    if HITS.fetch_add(1, Ordering::AcqRel) == 0 {
        let references = retained_references();
        assert_eq!(
            references, 2,
            "confirmation: Reference Release retains both references"
        );
        crate::putln!(
            "confirmation: reference release rejected from={} reference={reference} pages=6 references={references} ticks={}",
            from.get(),
            crate::machine::clock::now().as_ticks(),
        );
    }
    true
}

pub(crate) fn reference_released(from: TaskId, reference: usize) {
    if !crate::testing()
        || OP.load(Ordering::Relaxed) != injection::REFERENCE_RELEASE
        || from.get() != RETAINER.load(Ordering::Acquire)
        || reference != REFERENCE.load(Ordering::Acquire)
    {
        return;
    }
    assert!(
        !ENABLED.load(Ordering::Acquire),
        "confirmation: release bypassed injection"
    );
    assert!(
        !reference_owned(reference),
        "confirmation: successful release kept the handle"
    );
    let references = retained_references();
    assert_eq!(
        references, 1,
        "confirmation: only the table reference remains"
    );
    crate::putln!(
        "confirmation: reference released from={} reference={reference} references={references}",
        from.get(),
    );
}
