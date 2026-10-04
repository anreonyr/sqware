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
    assert!(meta.ready(HoleDir::Push));
    assert!(!meta.ready(HoleDir::Pull));
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
        assert!(!meta.ready(HoleDir::Push));
        assert!(matches!(ring(&meta), Err(MailFail::Busy)));
        assert!(matches!(
            reserve(&meta).and_then(|slot| slot.commit(Arc::new(alloc::vec![255]), TaskId::new(0))),
            Err(MailFail::Busy)
        ));
        for sequence in 0..QUEUE_CAP {
            assert!(meta.ready(HoleDir::Pull));
            take(&meta).unwrap();
            assert!(!meta.ready(HoleDir::Pull));
            assert!(matches!(take(&meta), Err(MailFail::Busy)));
            back(&meta);
            assert!(meta.ready(HoleDir::Pull));
            take(&meta).unwrap();
            let (sender, bytes) = source(&meta).unwrap();
            assert_eq!(sender, TaskId::new(sequence));
            assert_eq!(bytes.as_slice(), &[sequence as u8]);
            taken(&meta);
        }
        assert!(meta.ready(HoleDir::Push));
        assert!(!meta.ready(HoleDir::Pull));
        assert!(matches!(peek(&meta), Err(MailFail::Busy)));
        let current = storage(&meta);
        assert!(current.0 >= QUEUE_CAP);
        assert_eq!(*saved.get_or_insert(current), current);
        for _ in 0..RING_CAP {
            ring(&meta).unwrap();
        }
        assert_eq!(storage(&meta), current);
        assert!(!meta.ready(HoleDir::Push));
        assert!(meta.ready(HoleDir::Pull));
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
        assert!(meta.ready(HoleDir::Push));
        assert!(!meta.ready(HoleDir::Pull));
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
    assert!(!meta.ready(HoleDir::Pull));
    assert!(!meta.ready(HoleDir::Push));
    assert!(matches!(ring(&meta), Err(MailFail::Busy)));
    third
        .commit(Arc::new(alloc::vec![3]), TaskId::new(3))
        .unwrap();
    assert!(!meta.ready(HoleDir::Pull));
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
    assert!(!meta.ready(HoleDir::Push));
    assert!(!meta.ready(HoleDir::Pull));
    drop(fifth);
    assert!(meta.ready(HoleDir::Push));
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
    assert!(meta.ready(HoleDir::Pull));
    assert_eq!(receive(&meta), (TaskId::new(6), 6));
    assert!(meta.ready(HoleDir::Push));
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
    assert!(!meta.ready(HoleDir::Pull));
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
    assert!(!meta.ready(HoleDir::Push));
    assert!(!meta.ready(HoleDir::Pull));
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
