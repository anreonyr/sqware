use super::*;

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
    let meta = meta(TaskId::new(0));
    let live = HANDS_LIVE.load(Ordering::Relaxed);
    assert_eq!(storage(&meta).0, 0);
    assert!(meta.ready(MailCondition::Empty));
    assert!(!meta.ready(MailCondition::Pull));
    assert!(matches!(take(&meta), Err(MailFail::Busy)));
    assert!(matches!(hush(&meta), Err(MailFail::Busy)));
    let mut saved = None;
    for _ in 0..32 {
        for sequence in 0..QUEUE_CAP {
            reserve(&meta)
                .and_then(|slot| {
                    slot.commit(Arc::new(alloc::vec![sequence as u8]), TaskId::new(sequence))
                })
                .unwrap();
        }
        assert!(!meta.ready(MailCondition::Empty));
        assert!(matches!(ring(&meta), Err(MailFail::Busy)));
        assert!(matches!(
            reserve(&meta).and_then(|slot| slot.commit(Arc::new(alloc::vec![255]), TaskId::new(0))),
            Err(MailFail::Busy)
        ));
        for sequence in 0..QUEUE_CAP {
            assert!(meta.ready(MailCondition::Pull));
            take(&meta).unwrap();
            assert!(!meta.ready(MailCondition::Pull));
            assert!(matches!(take(&meta), Err(MailFail::Busy)));
            back(&meta);
            assert!(meta.ready(MailCondition::Pull));
            take(&meta).unwrap();
            let (sender, bytes) = source(&meta).unwrap();
            assert_eq!(sender, TaskId::new(sequence));
            assert_eq!(bytes.as_slice(), &[sequence as u8]);
            taken(&meta);
        }
        assert!(meta.ready(MailCondition::Empty));
        assert!(!meta.ready(MailCondition::Pull));
        assert!(matches!(peek(&meta), Err(MailFail::Busy)));
        let current = storage(&meta);
        assert!(current.0 >= QUEUE_CAP);
        assert_eq!(*saved.get_or_insert(current), current);
        for _ in 0..RING_CAP {
            ring(&meta).unwrap();
        }
        assert_eq!(storage(&meta), current);
        assert!(!meta.ready(MailCondition::Empty));
        assert!(meta.ready(MailCondition::Pull));
        assert!(matches!(take(&meta), Err(MailFail::Busy)));
        assert!(matches!(ring(&meta), Err(MailFail::Busy)));
        assert!(matches!(
            reserve(&meta).and_then(|slot| slot.commit(Arc::new(alloc::vec![255]), TaskId::new(0))),
            Err(MailFail::Busy)
        ));
        for _ in 0..RING_CAP {
            hush(&meta).unwrap();
        }
        assert_eq!(storage(&meta), current);
        assert!(meta.ready(MailCondition::Empty));
        assert!(!meta.ready(MailCondition::Pull));
        assert!(matches!(hush(&meta), Err(MailFail::Busy)));
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
        Err(MailFail::Dead)
    ));
    assert!(matches!(take(&meta), Err(MailFail::Dead)));
    seal(&meta);
    drop(meta);
    assert_eq!(HANDS_LIVE.load(Ordering::Relaxed), live);
    let rung = super::meta(TaskId::new(0));
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
    let meta = super::meta(TaskId::new(0));
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
    assert!(matches!(full, Err(MailFail::Busy)));
    assert!(!copied);
    assert!(!meta.ready(MailCondition::Pull));
    assert!(!meta.ready(MailCondition::Empty));
    assert!(matches!(ring(&meta), Err(MailFail::Busy)));
    third
        .commit(Arc::new(alloc::vec![3]), TaskId::new(3))
        .unwrap();
    assert!(!meta.ready(MailCondition::Pull));
    assert!(matches!(take(&meta), Err(MailFail::Busy)));
    assert!(matches!(peek(&meta), Err(MailFail::Busy)));
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
    assert!(!meta.ready(MailCondition::Empty));
    assert!(!meta.ready(MailCondition::Pull));
    drop(fifth);
    assert!(meta.ready(MailCondition::Empty));
    assert_eq!(HANDS_LIVE.load(Ordering::Relaxed), live);
    let failed = reserve(&meta).unwrap();
    reserve(&meta)
        .unwrap()
        .commit(Arc::new(alloc::vec![6]), TaskId::new(6))
        .unwrap();
    assert!(matches!(
        failed.commit(Arc::new(Vec::new()), TaskId::new(0)),
        Err(MailFail::Denied)
    ));
    assert!(meta.ready(MailCondition::Pull));
    assert_eq!(receive(&meta), (TaskId::new(6), 6));
    assert!(meta.ready(MailCondition::Empty));
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
    assert!(!meta.ready(MailCondition::Pull));
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
    assert!(!meta.ready(MailCondition::Empty));
    assert!(!meta.ready(MailCondition::Pull));
    let bytes = Arc::new(alloc::vec![8]);
    let released = Arc::downgrade(&bytes);
    assert!(matches!(
        inflight.commit(bytes, TaskId::new(8)),
        Err(MailFail::Dead)
    ));
    assert!(released.upgrade().is_none());
    drop(abandoned);
    seal(&meta);
    assert!(matches!(reserve(&meta), Err(MailFail::Dead)));
    drop(meta);
    assert_eq!(HANDS_LIVE.load(Ordering::Relaxed), live);
    crate::putln!(
        "hole: full reservation skips payload; out-of-order commit, cancellation and seal passed"
    );
}

pub fn discard_oversized() {
    let meta = meta(TaskId::new(0));
    let live = HANDS_LIVE.load(Ordering::Relaxed);
    assert!(matches!(read(&meta), Err(MailFail::Busy)));
    reserve_len(&meta, 65)
        .unwrap()
        .commit(Arc::new(alloc::vec![0; 65]), TaskId::new(1))
        .unwrap();
    reserve_len(&meta, 64)
        .unwrap()
        .commit(Arc::new(alloc::vec![7; 64]), TaskId::new(2))
        .unwrap();
    let reading = read(&meta).unwrap();
    assert!(matches!(read(&meta), Err(MailFail::Busy)));
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
    assert!(matches!(read(&meta), Err(MailFail::Busy)));
    drop(reserved);
    read(&meta).unwrap().finish();
    seal(&meta);
    assert!(matches!(read(&meta), Err(MailFail::Dead)));
    assert_eq!(HANDS_LIVE.load(Ordering::Relaxed), live);
}

pub fn limits() {
    let limits = HoleLimits {
        max_len: 4,
        max_messages: 2,
        max_bytes: 6,
    };
    let meta = try_meta_with_limits(TaskId::new(0), limits).unwrap();
    assert!(meta.ready(MailCondition::Push));
    let first = reserve_len(&meta, 4).unwrap();
    assert!(!meta.ready(MailCondition::Empty));
    assert!(meta.ready(MailCondition::Push));
    assert!(matches!(reserve_len(&meta, 5), Err(MailFail::Denied)));
    assert!(matches!(reserve_len(&meta, 3), Err(MailFail::Busy)));
    let second = reserve_len(&meta, 2).unwrap();
    assert!(!meta.ready(MailCondition::Push));
    drop(first);
    assert!(meta.ready(MailCondition::Push));
    let third = reserve_len(&meta, 4).unwrap();
    assert!(!meta.ready(MailCondition::Push));
    second
        .commit(Arc::new(alloc::vec![2; 2]), TaskId::new(2))
        .unwrap();
    third
        .commit(Arc::new(alloc::vec![3; 4]), TaskId::new(3))
        .unwrap();
    let reading = read(&meta).unwrap();
    assert!(!meta.ready(MailCondition::Push));
    drop(reading);
    read(&meta).unwrap().finish();
    assert!(meta.ready(MailCondition::Push));
    assert!(!meta.ready(MailCondition::Empty));
    read(&meta).unwrap().finish();
    assert!(meta.ready(MailCondition::Empty));
    let failed = reserve_len(&meta, 4).unwrap();
    assert!(matches!(
        failed.commit(Arc::new(alloc::vec![1; 3]), TaskId::new(0)),
        Err(MailFail::Denied)
    ));
    assert!(meta.ready(MailCondition::Empty));
    reserve_len(&meta, 4)
        .unwrap()
        .commit(Arc::new(alloc::vec![4; 4]), TaskId::new(4))
        .unwrap();
    read(&meta).unwrap().finish();
    ring(&meta).unwrap();
    assert!(!meta.ready(MailCondition::Push));
    hush(&meta).unwrap();
    assert!(meta.ready(MailCondition::Push));
    seal(&meta);
    assert!(!meta.ready(MailCondition::Push));
}

pub fn reading_guard() {
    let meta = meta(TaskId::new(0));
    let live = HANDS_LIVE.load(Ordering::Relaxed);
    reserve_len(&meta, 3)
        .unwrap()
        .commit(Arc::new(alloc::vec![1, 2, 3]), TaskId::new(2))
        .unwrap();
    let reading = read(&meta).unwrap();
    assert!(!meta.ready(MailCondition::Pull));
    drop(reading);
    assert!(meta.ready(MailCondition::Pull));
    let reading = read(&meta).unwrap();
    seal(&meta);
    assert_eq!(reading.bytes.as_slice(), &[1, 2, 3]);
    drop(reading);
    assert_eq!(HANDS_LIVE.load(Ordering::Relaxed), live);
    assert!(matches!(read(&meta), Err(MailFail::Dead)));
}

const QUEUE_CAP: usize = 4;
fn reserve(meta: &HoleMeta) -> Result<Reservation<'_>, MailFail> {
    reserve_len(meta, 0)
}

fn take(meta: &HoleMeta) -> Result<(), MailFail> {
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    let mut pending = meta.pending.lock();
    let Pending::Queue(q) = &mut *pending else {
        return Err(MailFail::Busy);
    };
    if q.taking || q.hands.front().and_then(Slot::hand).is_none() {
        return Err(MailFail::Busy);
    }
    // **字节归内核** ⇒ 这里不再有"发送方那段没了"那一档（取的一方也不必翻它的页表）。
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
