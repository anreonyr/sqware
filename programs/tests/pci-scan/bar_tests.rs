//! Ordinary RAM verifies slot classification and restoration, not PCI size masks.

use crate::ecam::{Ecam, BAR, HEADER};

#[test]
fn a_reset_zero_memory_bar_is_probed_and_its_registers_are_restored() {
    let mut memory = Box::new([0u32; 1024]);
    let ecam = Ecam::new(memory.as_mut_ptr() as usize, 0);
    let before = *memory;
    assert_eq!(ecam.bar_size(0), Some((16, false)));
    assert_eq!(*memory, before);
}

#[test]
fn bridge_nonbars_wide_final_slots_and_upper_halves_are_rejected() {
    let mut memory = Box::new([0u32; 1024]);
    let ecam = Ecam::new(memory.as_mut_ptr() as usize, 0);
    ecam.write16(HEADER, 1);
    ecam.write32(BAR + 8, 4);
    let before = *memory;
    assert_eq!(ecam.bar_size(2), None);
    assert_eq!(*memory, before);
    ecam.write16(HEADER, 0);
    ecam.write32(BAR + 20, 4);
    let before = *memory;
    assert_eq!(ecam.bar_size(5), None);
    assert_eq!(*memory, before);
    ecam.write32(BAR, 4);
    ecam.write32(BAR + 4, 4);
    let before = *memory;
    assert_eq!(ecam.bar_size(1), None);
    assert_eq!(*memory, before);
}

#[test]
fn a_wide_bars_upper_word_is_not_mistaken_for_another_bar_header() {
    let mut memory = Box::new([0u32; 1024]);
    let ecam = Ecam::new(memory.as_mut_ptr() as usize, 0);
    ecam.write32(BAR, 4);
    ecam.write32(BAR + 4, 4);
    assert_eq!(ecam.bar_size(2), Some((16, false)));
    ecam.write32(BAR, 5);
    ecam.write32(BAR + 4, 0);
    assert_eq!(ecam.bar_size(1), Some((16, false)), "I/O flags never consume an upper memory slot");
}

#[test]
fn unsupported_memory_types_and_headers_do_not_probe() {
    let mut memory = Box::new([0u32; 1024]);
    let ecam = Ecam::new(memory.as_mut_ptr() as usize, 0);
    for value in [1, 2, 6] {
        ecam.write32(BAR, value);
        let before = *memory;
        assert_eq!(ecam.bar_size(0), None);
        assert_eq!(*memory, before);
    }
    ecam.write16(HEADER, 2);
    ecam.write32(BAR, 4);
    assert_eq!(ecam.bar_size(0), None);
    assert_eq!(ecam.bar_size(6), None);
}
