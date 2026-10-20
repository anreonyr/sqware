use super::pending::RING_CAP;
use super::reading::{back, taken};
use super::reservation::Reservation;
use super::*;
use alloc::vec::Vec;

fn storage(meta: &HoleMeta) -> (usize, usize) {
    let pending = meta.pending.lock();
    let hands = match &*pending {
        Pending::Queue(q) => &q.hands,
        Pending::Rung { spare, .. } => spare,
        Pending::Dead => return (0, 0),
    };
    (hands.capacity(), hands.as_slices().0.as_ptr() as usize)
}

pub fn reuse() {
    let meta = HoleMeta::new(TaskId::new(0));
    let live = HANDS_LIVE.load(Ordering::Relaxed);
    assert_eq!(storage(&meta).0, 0);
    assert!(meta.ready(HoleCondition::Empty));
    assert!(!meta.ready(HoleCondition::Pull));
    assert!(matches!(take(&meta), Err(HoleFail::Busy)));
    assert!(matches!(hush(&meta), Err(HoleFail::Busy)));
    let mut saved = None;
    for _ in 0..32 {
        for sequence in 0..QUEUE_CAP {
            reserve(&meta)
                .and_then(|slot| {
                    slot.commit(Arc::new(alloc::vec![sequence as u8]), TaskId::new(sequence))
                })
                .unwrap();
        }
        assert!(!meta.ready(HoleCondition::Empty));
        assert!(matches!(ring(&meta), Err(HoleFail::Busy)));
        assert!(matches!(
            reserve(&meta).and_then(|slot| slot.commit(Arc::new(alloc::vec![255]), TaskId::new(0))),
            Err(HoleFail::Busy)
        ));
        for sequence in 0..QUEUE_CAP {
            assert!(meta.ready(HoleCondition::Pull));
            take(&meta).unwrap();
            assert!(!meta.ready(HoleCondition::Pull));
            assert!(matches!(take(&meta), Err(HoleFail::Busy)));
            back(&meta);
            assert!(meta.ready(HoleCondition::Pull));
            take(&meta).unwrap();
            let (sender, bytes) = source(&meta).unwrap();
            assert_eq!(sender, TaskId::new(sequence));
            assert_eq!(bytes.as_slice(), &[sequence as u8]);
            taken(&meta);
        }
        assert!(meta.ready(HoleCondition::Empty));
        assert!(!meta.ready(HoleCondition::Pull));
        assert!(matches!(peek(&meta), Err(HoleFail::Busy)));
        let current = storage(&meta);
        assert!(current.0 >= QUEUE_CAP);
        assert_eq!(*saved.get_or_insert(current), current);
        for _ in 0..RING_CAP {
            ring(&meta).unwrap();
        }
        assert_eq!(storage(&meta), current);
        assert!(!meta.ready(HoleCondition::Empty));
        assert!(meta.ready(HoleCondition::Pull));
        assert!(matches!(take(&meta), Err(HoleFail::Busy)));
        assert!(matches!(ring(&meta), Err(HoleFail::Busy)));
        assert!(matches!(
            reserve(&meta).and_then(|slot| slot.commit(Arc::new(alloc::vec![255]), TaskId::new(0))),
            Err(HoleFail::Busy)
        ));
        for _ in 0..RING_CAP {
            hush(&meta).unwrap();
        }
        assert_eq!(storage(&meta), current);
        assert!(meta.ready(HoleCondition::Empty));
        assert!(!meta.ready(HoleCondition::Pull));
        assert!(matches!(hush(&meta), Err(HoleFail::Busy)));
        assert_eq!(HANDS_LIVE.load(Ordering::Relaxed), live);
    }
    let bytes = Arc::new(alloc::vec![1]);
    let released = Arc::downgrade(&bytes);
    reserve(&meta)
        .unwrap()
        .commit(bytes, TaskId::new(0))
        .unwrap();
    seal(&meta);
    assert_eq!(storage(&meta).0, 0);
    assert!(released.upgrade().is_none());
    assert_eq!(HANDS_LIVE.load(Ordering::Relaxed), live);
    assert!(matches!(
        reserve(&meta).and_then(|slot| slot.commit(Arc::new(alloc::vec![1]), TaskId::new(0))),
        Err(HoleFail::Dead)
    ));
    assert!(matches!(take(&meta), Err(HoleFail::Dead)));
    seal(&meta);
    drop(meta);
    assert_eq!(HANDS_LIVE.load(Ordering::Relaxed), live);
    let rung = HoleMeta::new(TaskId::new(0));
    reserve(&rung)
        .and_then(|slot| slot.commit(Arc::new(alloc::vec![1]), TaskId::new(0)))
        .unwrap();
    take(&rung).unwrap();
    taken(&rung);
    ring(&rung).unwrap();
    assert!(storage(&rung).0 != 0);
    seal(&rung);
    assert_eq!(storage(&rung).0, 0);
    drop(rung);
    assert_eq!(HANDS_LIVE.load(Ordering::Relaxed), live);
    crate::putln!(
        "hole: FIFO storage reused through 32 signal cycles; seal releases storage and payloads"
    );
}

fn receive(meta: &HoleMeta) -> (TaskId, u8) {
    take(meta).unwrap();
    let (sender, bytes) = source(meta).unwrap();
    let byte = bytes[0];
    taken(meta);
    (sender, byte)
}

pub fn reservations() {
    let meta = HoleMeta::new(TaskId::new(0));
    let live = HANDS_LIVE.load(Ordering::Relaxed);
    let first = reserve(&meta).unwrap();
    let middle = reserve(&meta).unwrap();
    let third = reserve(&meta).unwrap();
    let fourth = reserve(&meta).unwrap();
    let mut copied = false;
    let full = reserve(&meta).and_then(|slot| {
        copied = true;
        slot.commit(Arc::new(alloc::vec![255]), TaskId::new(0))
    });
    assert!(matches!(full, Err(HoleFail::Busy)));
    assert!(!copied);
    assert!(!meta.ready(HoleCondition::Pull));
    assert!(!meta.ready(HoleCondition::Empty));
    assert!(matches!(ring(&meta), Err(HoleFail::Busy)));
    third
        .commit(Arc::new(alloc::vec![3]), TaskId::new(3))
        .unwrap();
    assert!(!meta.ready(HoleCondition::Pull));
    assert!(matches!(take(&meta), Err(HoleFail::Busy)));
    assert!(matches!(peek(&meta), Err(HoleFail::Busy)));
    drop(middle);
    let fifth = reserve(&meta).unwrap();
    fourth
        .commit(Arc::new(alloc::vec![4]), TaskId::new(4))
        .unwrap();
    first
        .commit(Arc::new(alloc::vec![1]), TaskId::new(1))
        .unwrap();
    assert_eq!(peek(&meta).unwrap(), (1, TaskId::new(1), 3));
    for sequence in [1, 3, 4] {
        assert_eq!(receive(&meta), (TaskId::new(sequence), sequence as u8));
    }
    assert!(!meta.ready(HoleCondition::Empty));
    assert!(!meta.ready(HoleCondition::Pull));
    drop(fifth);
    assert!(meta.ready(HoleCondition::Empty));
    assert_eq!(HANDS_LIVE.load(Ordering::Relaxed), live);
    let failed = reserve(&meta).unwrap();
    reserve(&meta)
        .unwrap()
        .commit(Arc::new(alloc::vec![6]), TaskId::new(6))
        .unwrap();
    assert!(matches!(
        failed.commit(Arc::new(Vec::new()), TaskId::new(0)),
        Err(HoleFail::Denied)
    ));
    assert!(meta.ready(HoleCondition::Pull));
    assert_eq!(receive(&meta), (TaskId::new(6), 6));
    assert!(meta.ready(HoleCondition::Empty));
    reserve(&meta)
        .unwrap()
        .commit(Arc::new(alloc::vec![9]), TaskId::new(9))
        .unwrap();
    let middle = reserve(&meta).unwrap();
    reserve(&meta)
        .unwrap()
        .commit(Arc::new(alloc::vec![10]), TaskId::new(10))
        .unwrap();
    take(&meta).unwrap();
    let (sender, bytes) = source(&meta).unwrap();
    drop(middle);
    assert!(!meta.ready(HoleCondition::Pull));
    assert_eq!(sender, TaskId::new(9));
    assert_eq!(bytes.as_slice(), &[9]);
    taken(&meta);
    assert_eq!(receive(&meta), (TaskId::new(10), 10));
    let inflight = reserve(&meta).unwrap();
    let abandoned = reserve(&meta).unwrap();
    let bytes = Arc::new(alloc::vec![7]);
    let released = Arc::downgrade(&bytes);
    reserve(&meta)
        .unwrap()
        .commit(bytes, TaskId::new(7))
        .unwrap();
    seal(&meta);
    assert!(released.upgrade().is_none());
    assert_eq!(HANDS_LIVE.load(Ordering::Relaxed), live);
    assert!(!meta.ready(HoleCondition::Empty));
    assert!(!meta.ready(HoleCondition::Pull));
    let bytes = Arc::new(alloc::vec![8]);
    let released = Arc::downgrade(&bytes);
    assert!(matches!(
        inflight.commit(bytes, TaskId::new(8)),
        Err(HoleFail::Dead)
    ));
    assert!(released.upgrade().is_none());
    drop(abandoned);
    seal(&meta);
    assert!(matches!(reserve(&meta), Err(HoleFail::Dead)));
    drop(meta);
    assert_eq!(HANDS_LIVE.load(Ordering::Relaxed), live);
    crate::putln!(
        "hole: full reservation skips payload; out-of-order commit, cancellation and seal passed"
    );
}

pub fn discard_oversized() {
    let meta = HoleMeta::new(TaskId::new(0));
    let live = HANDS_LIVE.load(Ordering::Relaxed);
    assert!(matches!(read(&meta), Err(HoleFail::Busy)));
    reserve_len(&meta, 65)
        .unwrap()
        .commit(Arc::new(alloc::vec![0; 65]), TaskId::new(1))
        .unwrap();
    reserve_len(&meta, 64)
        .unwrap()
        .commit(Arc::new(alloc::vec![7; 64]), TaskId::new(2))
        .unwrap();
    let reading = read(&meta).unwrap();
    assert!(matches!(read(&meta), Err(HoleFail::Busy)));
    assert_eq!(reading.bytes.len(), 65);
    drop(reading);
    assert_eq!(peek(&meta).unwrap().0, 65);
    read(&meta).unwrap().finish();
    assert_eq!(peek(&meta).unwrap(), (64, TaskId::new(2), 1));
    let reading = read(&meta).unwrap();
    assert_eq!(reading.from, TaskId::new(2));
    assert_eq!(reading.bytes.as_slice(), &[7; 64]);
    drop(reading);
    assert_eq!(peek(&meta).unwrap().0, 64);
    read(&meta).unwrap().finish();
    assert_eq!(HANDS_LIVE.load(Ordering::Relaxed), live);
    let reserved = reserve_len(&meta, 1).unwrap();
    reserve_len(&meta, 65)
        .unwrap()
        .commit(Arc::new(alloc::vec![0; 65]), TaskId::new(3))
        .unwrap();
    assert!(matches!(read(&meta), Err(HoleFail::Busy)));
    drop(reserved);
    read(&meta).unwrap().finish();
    seal(&meta);
    assert!(matches!(read(&meta), Err(HoleFail::Dead)));
    assert_eq!(HANDS_LIVE.load(Ordering::Relaxed), live);
}

pub fn limits() {
    let limits = HoleLimits {
        max_len: 4,
        max_messages: 2,
        max_bytes: 6,
    };
    let meta = HoleMeta::try_new_with_limits(TaskId::new(0), limits).unwrap();
    assert!(meta.ready(HoleCondition::Push));
    let first = reserve_len(&meta, 4).unwrap();
    assert!(!meta.ready(HoleCondition::Empty));
    assert!(meta.ready(HoleCondition::Push));
    assert!(matches!(reserve_len(&meta, 5), Err(HoleFail::Denied)));
    assert!(matches!(reserve_len(&meta, 3), Err(HoleFail::Busy)));
    let second = reserve_len(&meta, 2).unwrap();
    assert!(!meta.ready(HoleCondition::Push));
    drop(first);
    assert!(meta.ready(HoleCondition::Push));
    let third = reserve_len(&meta, 4).unwrap();
    assert!(!meta.ready(HoleCondition::Push));
    second
        .commit(Arc::new(alloc::vec![2; 2]), TaskId::new(2))
        .unwrap();
    third
        .commit(Arc::new(alloc::vec![3; 4]), TaskId::new(3))
        .unwrap();
    let reading = read(&meta).unwrap();
    assert!(!meta.ready(HoleCondition::Push));
    drop(reading);
    read(&meta).unwrap().finish();
    assert!(meta.ready(HoleCondition::Push));
    assert!(!meta.ready(HoleCondition::Empty));
    read(&meta).unwrap().finish();
    assert!(meta.ready(HoleCondition::Empty));
    let failed = reserve_len(&meta, 4).unwrap();
    assert!(matches!(
        failed.commit(Arc::new(alloc::vec![1; 3]), TaskId::new(0)),
        Err(HoleFail::Denied)
    ));
    assert!(meta.ready(HoleCondition::Empty));
    reserve_len(&meta, 4)
        .unwrap()
        .commit(Arc::new(alloc::vec![4; 4]), TaskId::new(4))
        .unwrap();
    read(&meta).unwrap().finish();
    ring(&meta).unwrap();
    assert!(!meta.ready(HoleCondition::Push));
    hush(&meta).unwrap();
    assert!(meta.ready(HoleCondition::Push));
    seal(&meta);
    assert!(!meta.ready(HoleCondition::Push));
}

pub fn reading_guard() {
    let meta = HoleMeta::new(TaskId::new(0));
    let live = HANDS_LIVE.load(Ordering::Relaxed);
    reserve_len(&meta, 3)
        .unwrap()
        .commit(Arc::new(alloc::vec![1, 2, 3]), TaskId::new(2))
        .unwrap();
    let reading = read(&meta).unwrap();
    assert!(!meta.ready(HoleCondition::Pull));
    drop(reading);
    assert!(meta.ready(HoleCondition::Pull));
    let reading = read(&meta).unwrap();
    seal(&meta);
    assert_eq!(reading.bytes.as_slice(), &[1, 2, 3]);
    drop(reading);
    assert_eq!(HANDS_LIVE.load(Ordering::Relaxed), live);
    assert!(matches!(read(&meta), Err(HoleFail::Dead)));
}

const QUEUE_CAP: usize = 4;
fn reserve(meta: &HoleMeta) -> Result<Reservation<'_>, HoleFail> {
    reserve_len(meta, 0)
}

fn take(meta: &HoleMeta) -> Result<(), HoleFail> {
    if !meta.alive() {
        return Err(HoleFail::Dead);
    }
    let mut pending = meta.pending.lock();
    let Pending::Queue(q) = &mut *pending else {
        return Err(HoleFail::Busy);
    };
    if q.taking || q.hands.front().and_then(Slot::hand).is_none() {
        return Err(HoleFail::Busy);
    }
    q.taking = true;
    Ok(())
}

fn source(meta: &HoleMeta) -> Option<(TaskId, Arc<Vec<u8>>)> {
    let pending = meta.pending.lock();
    match &*pending {
        Pending::Queue(q) if q.taking => q
            .hands
            .front()
            .and_then(Slot::hand)
            .map(|h| (h.from, h.buf.clone())),
        _ => None,
    }
}
