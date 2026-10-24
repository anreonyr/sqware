use crate::driver::pci::ecam::{self, Ecam};

pub fn zero32(ecam: &Ecam) {
    assert_eq!(ecam.read16(ecam::COMMAND) & ecam::COMMAND_MASTER, 0);
    let mut found = false;
    for index in 0..6 {
        let register = ecam::BAR + u16::from(index) * 4;
        if ecam.read32(register) != 0 { continue; }
        let Some((size, wide)) = ecam.bar_size(index) else { continue; };
        assert!(!wide);
        assert!(size >= 16);
        assert_eq!(ecam.read32(register), 0, "actual BAR is restored after sizing");
        assert_eq!(ecam.read16(ecam::COMMAND) & ecam::COMMAND_MASTER, 0);
        crate::debug::put(&alloc::format!(
            "pci-probe: reset-zero requester={} bar={index} size={size:#x} restored=0 master=0",
            ecam.requester(),
        ));
        found = true;
    }
    assert!(found, "native fixture needs an implemented reset-zero 32-bit BAR");
}

pub fn upper(ecam: &Ecam, index: u8) {
    if (ecam.read32(ecam::BAR + u16::from(index) * 4) >> 1) & 3 != 2 { return; }
    let upper = index + 1;
    let before = ecam.read32(ecam::BAR + u16::from(upper) * 4);
    let guard = ecam.read32(0x28);
    assert_eq!(ecam.bar_size(upper), None);
    assert_eq!(ecam.read32(ecam::BAR + u16::from(upper) * 4), before);
    assert_eq!(ecam.read32(0x28), guard);
    crate::debug::put(&alloc::format!(
        "pci-probe: upper-half requester={} bar={upper} rejected=true restored=true",
        ecam.requester(),
    ));
}
