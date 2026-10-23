use crate::end::End;
use crate::function::Function;
use crate::memory::Memory;
use crate::scan::{scan, WORK};
use crate::snapshot::{Snapshot, LEN};
use crate::sparse::Sparse;
use crate::window::{EcamWindow, FUNCTION_SIZE};

fn function() -> Function {
    Function {
        requester: 0,
        vendor: 0,
        device: 0,
        class: 0,
    }
}

fn write16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn present(bytes: &mut [u8], requester: u16, vendor: u16, device: u16, class: u32) {
    let at = requester as usize * FUNCTION_SIZE;
    write16(bytes, at, vendor);
    write16(bytes, at + 2, device);
    bytes[at + 9] = class as u8;
    bytes[at + 10] = (class >> 8) as u8;
    bytes[at + 11] = (class >> 16) as u8;
}

#[test]
fn actual_window_snapshot_is_exactly_256_bytes_and_bounded() {
    let mut memory = Memory::new(FUNCTION_SIZE * 3, 0xFF);
    present(memory.bytes_mut(), 1, 0x1AF4, 0x1042, 0x01_02_03);
    let bytes = memory.bytes();
    let base = memory.base();
    // SAFETY: the buffer remains readable and allocated through all window uses.
    let window = unsafe { EcamWindow::new(base, bytes.len()) }.unwrap();

    assert_eq!(window.limit(), 3);

    let snapshot = Snapshot::read(&window, 1).unwrap();
    assert_eq!(snapshot.bytes().len(), LEN);
    assert_eq!(snapshot.vendor(), 0x1AF4);
    assert_eq!(snapshot.device(), 0x1042);
    assert_eq!(snapshot.class(), 0x01_02_03);

    assert!(Snapshot::read(&window, 2).is_some());
    assert!(Snapshot::read(&window, 3).is_none());
    assert_eq!(window.read8(3, 0), None);
    assert_eq!(window.read8(0, 4096), None);
    assert_eq!(window.read8(2, 4095), Some(0xFF));
}

#[test]
fn actual_window_handles_short_ranges_and_rejects_unaligned_bases_and_overflow() {
    let memory = Memory::new(FUNCTION_SIZE * 3 + 1, 0);
    let base = memory.base();

    // SAFETY: all accepted ranges stay inside the live byte buffer.
    assert!(unsafe { EcamWindow::new(base, 0) }.is_none());
    assert_eq!(
        unsafe { EcamWindow::new(base, FUNCTION_SIZE - 1) }.unwrap().limit(),
        0
    );
    assert!(unsafe { EcamWindow::new(usize::MAX - (FUNCTION_SIZE - 1), FUNCTION_SIZE) }.is_none());

    assert!(unsafe { EcamWindow::new(base + 1, FUNCTION_SIZE * 3) }.is_none());
}

#[test]
fn actual_partial_window_eof_is_not_65536() {
    let memory = Memory::new(FUNCTION_SIZE * 3, 0xFF);
    let bytes = memory.bytes();
    // SAFETY: both windows remain within the buffer, which outlives their reads.
    let window = unsafe { EcamWindow::new(bytes.as_ptr() as usize, bytes.len()) }.unwrap();
    let mut out = [function(); 4];

    let page = scan(&window, 0, &mut out).unwrap();
    assert_eq!(page.next, 3);
    assert_eq!(page.count, 0);
    assert_eq!(page.end, End::Eof);

    assert_eq!(scan(&window, 99, &mut out), None);
    let end = scan(&window, 3, &mut out).unwrap();
    assert_eq!(end.next, 3);
    assert_eq!(end.count, 0);
    assert_eq!(end.end, End::Eof);

    let short = unsafe { EcamWindow::new(bytes.as_ptr() as usize, FUNCTION_SIZE - 1) }.unwrap();
    let page = scan(&short, 0, &mut out).unwrap();
    assert_eq!(page.next, 0);
    assert_eq!(page.count, 0);
    assert_eq!(page.end, End::Eof);
}

#[test]
fn sparse_capacity_continuation_keeps_first_unscanned_bdf() {
    let mut sparse = Sparse::new(0x1_0000);
    sparse.present(0, 0x1AF4, 0x1042, 0x010000);
    sparse.present(1, 0x1AF4, 0x1001, 0x010000);
    let mut out = [function(); 1];

    let first = scan(&sparse, 0, &mut out).unwrap();
    assert_eq!(first.end, End::Capacity);
    assert_eq!(first.next, 1);
    assert_eq!(first.count, 1);
    assert_eq!(out[0].requester, 0);

    let second = scan(&sparse, first.next, &mut out).unwrap();
    assert_eq!(second.end, End::Capacity);
    assert_eq!(second.next, 2);
    assert_eq!(second.count, 1);
    assert_eq!(out[0].requester, 1);

    let third = scan(&sparse, second.next, &mut out).unwrap();
    assert_eq!(third.end, End::WorkLimit);
    assert_eq!(third.next, 2 + WORK);
    assert_eq!(third.count, 0);
}

#[test]
fn sparse_empty_work_window_continues_instead_of_eof() {
    let sparse = Sparse::new(0x1_0000);
    let mut out = [function(); 4];

    let first = scan(&sparse, 0, &mut out).unwrap();
    assert_eq!(first.end, End::WorkLimit);
    assert_eq!(first.next, WORK);
    assert_eq!(first.count, 0);

    let second = scan(&sparse, first.next, &mut out).unwrap();
    assert_eq!(second.end, End::WorkLimit);
    assert_eq!(second.next, WORK * 2);
    assert_eq!(second.count, 0);
}

#[test]
fn sparse_high_bus_jump_reaches_last_requester_and_eof() {
    let mut sparse = Sparse::new(0x1_0000);
    sparse.present(0xFFFF, 0x1AF4, 0x1042, 0x12_34_56);
    let mut out = [function(); 1];

    let page = scan(&sparse, 0xFF00, &mut out).unwrap();
    assert_eq!(page.end, End::Eof);
    assert_eq!(page.next, 0x1_0000);
    assert_eq!(page.count, 1);
    assert_eq!(out[0].requester, 0xFFFF);
    assert_eq!(out[0].class, 0x12_34_56);
}

#[test]
fn snapshot_reads_exactly_256_bytes_and_class_is_24_bit() {
    let mut sparse = Sparse::new(1);
    sparse.present(0, 0x1AF4, 0x1042, 0x12_34_56);

    let snapshot = Snapshot::read(&sparse, 0).unwrap();
    assert_eq!(sparse.reads(), 256);
    assert_eq!(snapshot.bytes().len(), LEN);
    assert_eq!(snapshot.class(), 0x12_34_56);
    assert_eq!(snapshot.class() & !0x00FF_FFFF, 0);
    assert!(Snapshot::read(&sparse, 1).is_none());
}

#[test]
fn scan_and_snapshot_do_not_modify_the_mapping() {
    let mut memory = Memory::new(FUNCTION_SIZE * 2 + 127, 0xFF);
    present(memory.bytes_mut(), 1, 0xABCD, 0x1234, 0x56_78_9A);
    let bytes = memory.bytes();
    let before = bytes.to_vec();
    // SAFETY: the window is used only while the unchanged buffer remains live.
    let window = unsafe { EcamWindow::new(bytes.as_ptr() as usize, bytes.len()) }.unwrap();
    let mut out = [function(); 4];
    let page = scan(&window, 0, &mut out).unwrap();
    assert_eq!(page.end, End::Eof);
    assert_eq!(page.next, 2);
    assert_eq!(page.count, 1);
    assert_eq!(out[0].class, 0x56_78_9A);
    assert!(Snapshot::read(&window, 1).unwrap().valid());
    assert!(Snapshot::read(&window, 2).is_none());
    assert_eq!(bytes, before);
}

#[test]
fn invalid_cursor_and_invalid_reader_geometry_do_not_probe_mmio() {
    let mut out = [function(); 1];
    let small = Sparse::new(3);
    assert_eq!(scan(&small, 4, &mut out), None);
    assert_eq!(small.reads(), 0);
    let oversized = Sparse::new(0x1_0001);
    assert_eq!(scan(&oversized, 0, &mut out), None);
    assert_eq!(Snapshot::read(&oversized, 0), None);
    assert_eq!(oversized.reads(), 0);
}

#[test]
fn a_read_failure_is_not_skipped_as_an_absent_function() {
    for register in [0, 1, 2, 3, 9, 10, 11] {
        let mut sparse = Sparse::new(2);
        sparse.present(0, 0xABCD, 0x1234, 0x56_78_9A);
        sparse.present(1, 0xABCD, 0x1235, 0x56_78_9B);
        sparse.fail(1, register);
        let mut out = [function(); 4];
        assert_eq!(scan(&sparse, 0, &mut out), None, "register {register}");
        assert_eq!(Snapshot::read(&sparse, 1), None);
    }
}

#[test]
fn no_output_capacity_reports_continuation_without_reading() {
    let sparse = Sparse::new(9);
    let page = scan(&sparse, 3, &mut []).unwrap();
    assert_eq!(page.end, End::Capacity);
    assert_eq!(page.next, 3);
    assert_eq!(page.count, 0);
    assert_eq!(sparse.reads(), 0);
}

#[test]
fn snapshot_propagates_failure_at_the_last_byte() {
    let mut sparse = Sparse::new(1);
    sparse.present(0, 0xABCD, 0x1234, 0x56_78_9A);
    sparse.fail(0, 255);
    assert_eq!(Snapshot::read(&sparse, 0), None);
    assert_eq!(sparse.reads(), 256);
}
