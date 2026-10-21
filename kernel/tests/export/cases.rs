use super::*;
use crate::{fs, kind::{EventKind, HaltEvent, RoomEvent}, machine};
use std::collections::BTreeSet;

fn reset() {
    assert_eq!(machine::sync::BUSY.load(Ordering::Relaxed), 0);
    *HOST.try_lock().unwrap() = None;
    FAILURE.store(0, Ordering::Release);
    machine::clock::TICKS.store(0, Ordering::Relaxed);
    fs::CREATE_FAIL.store(false, Ordering::Relaxed);
    fs::CREATE_CALLS.store(0, Ordering::Relaxed);
    fs::WRITE_FAIL_AT.store(usize::MAX, Ordering::Relaxed);
    fs::WRITE_CALLS.store(0, Ordering::Relaxed);
    fs::BYTES.lock().unwrap().clear();
}

fn observed_failure() -> Option<Failure> {
    Failure::decode(FAILURE.load(Ordering::Acquire))
}

#[test]
fn export_preserves_all_records_or_latches_the_first_real_failure() {
    reset();
    machine::sync::BUSY.store(4, Ordering::Relaxed);
    note_sink(EventKind::Room(RoomEvent::Spawn { tid: 7 }), 2, 123);
    assert!(machine::clock::TICKS.load(Ordering::Relaxed) > HOST_WAIT_TICKS);
    assert!(complete(), "ordinary lock contention is not evidence loss");
    let rows = fs::BYTES.lock().unwrap().clone();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&rows).unwrap(),
        serde_json::json!({"h":2,"when":123,"kind":{"room":{"spawn":{"tid":7}}}}),
    );
    assert!(rows.ends_with(b"\n"));

    reset();
    std::thread::scope(|scope| {
        for hart in 0..4 {
            scope.spawn(move || {
                for id in 0..1000 {
                    note_sink(EventKind::Room(RoomEvent::Spawn { tid: hart * 1000 + id }), hart, id as u64);
                }
            });
        }
    });
    assert!(complete());
    let rows = String::from_utf8(fs::BYTES.lock().unwrap().clone()).unwrap();
    let ids: BTreeSet<_> = rows.lines().map(|line| {
        let row: serde_json::Value = serde_json::from_str(line).unwrap();
        let id = row["kind"]["room"]["spawn"]["tid"].as_u64().unwrap();
        assert_eq!(row["h"].as_u64().unwrap(), id / 1000);
        id
    }).collect();
    assert_eq!(rows.lines().count(), 4000);
    assert_eq!(ids, (0..4000).collect());
    assert_eq!(fs::CREATE_CALLS.load(Ordering::Relaxed), 1);

    reset();
    fs::WRITE_FAIL_AT.store(2, Ordering::Relaxed);
    note_sink(EventKind::Room(RoomEvent::Idle), 0, 1);
    assert!(!complete(), "a failed newline must reject the entire export");
    assert_eq!(observed_failure(), Some(Failure::Write));
    assert!(!fs::BYTES.lock().unwrap().ends_with(b"\n"));
    let writes = fs::WRITE_CALLS.load(Ordering::Relaxed);
    fail(Failure::Encode);
    note_sink(EventKind::Room(RoomEvent::Idle), 0, 2);
    assert_eq!(observed_failure(), Some(Failure::Write));
    assert_eq!(fs::WRITE_CALLS.load(Ordering::Relaxed), writes);

    reset();
    fs::CREATE_FAIL.store(true, Ordering::Relaxed);
    note_sink(EventKind::Room(RoomEvent::Idle), 0, 1);
    assert_eq!(observed_failure(), Some(Failure::Create));
    assert!(!complete());
    fs::CREATE_FAIL.store(false, Ordering::Relaxed);
    note_sink(EventKind::Room(RoomEvent::Idle), 0, 2);
    assert_eq!(fs::CREATE_CALLS.load(Ordering::Relaxed), 1);
    assert!(fs::BYTES.lock().unwrap().is_empty());

    reset();
    {
        let _stopped_owner = HOST.try_lock().unwrap();
        machine::clock::TICKS.store(u64::MAX - 5000, Ordering::Relaxed);
        note_sink(EventKind::Halt(HaltEvent::Panic), 0, 3);
        assert_eq!(observed_failure(), Some(Failure::LockTimeout));
        assert!(!complete(), "panic export must not wait on a stopped lock owner");
    }
    assert!(fs::BYTES.lock().unwrap().is_empty());

    reset();
    {
        let _stopped_owner = HOST.try_lock().unwrap();
        export(&Report::default());
        assert_eq!(observed_failure(), Some(Failure::LockTimeout));
        assert!(!complete());
    }

    reset();
    fail(Failure::Encode);
    fail(Failure::Create);
    assert_eq!(observed_failure(), Some(Failure::Encode));
    assert!(!complete());
}
