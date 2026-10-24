use crate::{pci_api::Fail, pci_core::{Core, PlaceRequest}, platform::Window};
use crate::{PAGE_SIZE, PieToken, TaskId, state};

fn core() -> Core {
    Core::new(0x3000_0000, 2, Window {
        token: state::APERTURE, base: 0x4_0000_0000_0000, size: 4 * 0x4000,
    }).unwrap()
}

fn request(serial: u64, requester: u32, bar: u8) -> PlaceRequest {
    PlaceRequest { serial, requester, recipient: TaskId::new(9), bar }
}

#[test]
fn an_empty_bar_row_blocks_enable_even_when_another_bar_is_published() {
    state::reset();
    let mut core = core();
    assert_ne!(core.place(request(1, 1, 0)).unwrap().bar, PieToken::NONE);
    assert_eq!(core.place(request(1, 1, 1)).err(), Some(Fail::Denied));
    assert_eq!(core.enable(1, 1), Err(Fail::Conflict));
    core.disable(1, 1).unwrap();
    core.release(1, 1).unwrap();
    core.place(request(2, 1, 0)).unwrap();
    core.enable(2, 1).unwrap();
}

#[test]
fn two_functions_have_distinct_grants_and_independent_master_and_serial_floors() {
    state::reset();
    let mut core = core();
    let first = core.place(request(7, 0, 0)).unwrap();
    let (address, pages, grant) = (first.address, first.pages, first.bar);
    let second = core.place(request(7, 1, 0)).unwrap();
    assert!(second.address >= address + u64::from(pages) * PAGE_SIZE as u64);
    assert_ne!(second.bar, grant);
    assert_eq!(state::derives(), 2);
    core.enable(7, 0).unwrap();
    core.enable(7, 1).unwrap();
    core.disable(7, 0).unwrap();
    assert_eq!(crate::ecam::Ecam::new(0x3000_0000, 1).read16(crate::ecam::COMMAND)
        & crate::ecam::COMMAND_MASTER, crate::ecam::COMMAND_MASTER);
    core.release(7, 0).unwrap();
    assert_eq!(core.place(request(7, 0, 0)).err(), Some(Fail::Conflict));
    core.place(request(8, 0, 0)).unwrap();
    core.release(7, 0).unwrap();
    core.enable(8, 0).unwrap();
    core.enable(7, 1).unwrap();
    assert_eq!(state::derives(), 2, "published roots are reused per function");
}
