use super::*;

fn storage(meta: &HoleMeta) -> (usize, usize) {
    let pending = meta.pending.lock();
    let hands = match &*pending {
        Pending::Queue(q) => &q.hands,
        Pending::Rung { spare, .. } => spare,
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
            give(&meta, Arc::new(alloc::vec![sequence as u8]), TaskId::new(sequence)).unwrap();
        }
        assert!(!meta.ready(HoleDir::Push));
        assert!(matches!(ring(&meta), Err(MailFail::Busy)));
        assert!(matches!(give(&meta, Arc::new(alloc::vec![255]), TaskId::new(0)), Err(MailFail::Busy)));
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
        for _ in 0..RING_CAP { ring(&meta).unwrap(); }
        assert_eq!(storage(&meta), current);
        assert!(!meta.ready(HoleDir::Push));
        assert!(meta.ready(HoleDir::Pull));
        assert!(matches!(take(&meta), Err(MailFail::Busy)));
        assert!(matches!(ring(&meta), Err(MailFail::Busy)));
        assert!(matches!(give(&meta, Arc::new(alloc::vec![255]), TaskId::new(0)), Err(MailFail::Busy)));
        for _ in 0..RING_CAP { hush(&meta).unwrap(); }
        assert_eq!(storage(&meta), current);
        assert!(meta.ready(HoleDir::Push));
        assert!(!meta.ready(HoleDir::Pull));
        assert!(matches!(hush(&meta), Err(MailFail::Busy)));
        assert_eq!(HANDS_LIVE.load(Ordering::Relaxed), live);
    }
    let bytes = Arc::new(alloc::vec![1]);
    let released = Arc::downgrade(&bytes);
    give(&meta, bytes, TaskId::new(0)).unwrap();
    seal(&meta);
    assert_eq!(storage(&meta).0, 0);
    assert!(released.upgrade().is_none());
    assert_eq!(HANDS_LIVE.load(Ordering::Relaxed), live);
    assert!(matches!(give(&meta, Arc::new(alloc::vec![1]), TaskId::new(0)), Err(MailFail::Dead)));
    assert!(matches!(take(&meta), Err(MailFail::Dead)));
    seal(&meta);
    drop(meta);
    assert_eq!(HANDS_LIVE.load(Ordering::Relaxed), live);
    let rung = super::meta(TaskId::new(0));
    give(&rung, Arc::new(alloc::vec![1]), TaskId::new(0)).unwrap();
    take(&rung).unwrap();
    taken(&rung);
    ring(&rung).unwrap();
    assert!(storage(&rung).0 != 0);
    seal(&rung);
    assert_eq!(storage(&rung).0, 0);
    drop(rung);
    assert_eq!(HANDS_LIVE.load(Ordering::Relaxed), live);
    crate::putln!("hole: FIFO storage reused through 32 signal cycles; seal releases storage and payloads");
}
