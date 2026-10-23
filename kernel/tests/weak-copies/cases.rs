use crate::copies::{self, Origin};
use crate::machine::hart;
use std::sync::{Barrier, Mutex};

static SERIAL: Mutex<()> = Mutex::new(());

#[test]
fn held_copies_are_detected_only_on_their_own_hart() {
    let _serial = SERIAL.lock().unwrap();
    for origin in [Origin::Muster, Origin::Snapshot] {
        hart::select(31);
        let slot = copies::register(origin);
        assert_ne!(slot, 0);
        hart::select(30);
        copies::check_block_heldout();
        hart::select(31);
        let held = std::panic::catch_unwind(copies::check_block_heldout);
        copies::release(slot);
        assert!(held.is_err());
        copies::check_block_heldout();
    }
}

#[test]
fn concurrent_slot_reuse_never_reports_a_copy_on_another_hart() {
    let _serial = SERIAL.lock().unwrap();
    let start = Barrier::new(8);
    std::thread::scope(|threads| {
        for id in 0..8 {
            let start = &start;
            threads.spawn(move || {
                hart::select(id);
                start.wait();
                for n in 0..200_000 {
                    let origin = if n % 2 == 0 { Origin::Muster } else { Origin::Snapshot };
                    let slot = copies::register(origin);
                    assert_ne!(slot, 0);
                    std::hint::spin_loop();
                    copies::release(slot);
                    copies::check_block_heldout();
                }
            });
        }
    });
}

#[test]
fn stored_copies_do_not_mask_a_temporary_copy_after_slot_reuse() {
    let _serial = SERIAL.lock().unwrap();
    hart::select(29);
    for origin in [Origin::Roster, Origin::Holder, Origin::TeamTasks, Origin::Sire] {
        let stored = copies::register(origin);
        assert_ne!(stored, 0);
        copies::check_block_heldout();
        copies::release(stored);
        let temporary = copies::register(Origin::Muster);
        assert_ne!(temporary, 0);
        let held = std::panic::catch_unwind(copies::check_block_heldout);
        copies::release(temporary);
        assert!(held.is_err());
    }
}
