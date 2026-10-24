use crate::{ecam, pci_api::Fail, pci_core::{Core, PlaceRequest}, platform::Window};
use crate::{PAGE_SIZE, PieToken, TaskId, state};

fn core() -> Core {
    Core::new(0x3000_0000, 2, Window {
        token: state::APERTURE, base: 0x4_0000_0000_0000, size: 4 * 0x4000,
    }).unwrap()
}

fn full_core() -> Core {
    state::set_aperture_pages(state::BAR_SIZE as usize / PAGE_SIZE);
    Core::new(0x3000_0000, 2, Window {
        token: state::APERTURE, base: 0x4_0000_0000_0000, size: state::BAR_SIZE,
    }).unwrap()
}

fn request(serial: u64, requester: u32, bar: u8) -> PlaceRequest {
    PlaceRequest { serial, requester, recipient: TaskId::new(9), bar }
}

#[test]
fn a_subpage_bar_reserves_a_whole_page_and_cannot_share_it_with_another_bar() {
    state::reset();
    state::set_aperture_pages(1);
    state::set_bar_size(0, 2048);
    state::set_bar_size(1, 2048);
    let mut core = Core::new(0x3000_0000, 2, Window {
        token: state::APERTURE, base: 0x4_0000_0000_0000, size: PAGE_SIZE as u64,
    }).unwrap();
    let first = core.place(request(1, 0, 0)).unwrap();
    assert_eq!(first.size, 2048);
    assert_eq!(first.pages, 1);
    assert_eq!(state::derived().3, crate::GrantRange::Pages { offset: 0, count: 1 });
    assert!(state::unseals().is_empty(), "one page cannot be a strict child of itself");
    core.enable(1, 0).unwrap();
    assert_eq!(core.place(request(2, 1, 0)).err(), Some(Fail::Denied));
    assert_eq!(state::derives(), 1, "page isolation prevents a second subpage owner");
}

#[test]
fn an_unpublished_full_root_release_clears_the_exclusive_parent_heir() {
    state::reset();
    let mut core = full_core();
    state::fail_next_accord();
    assert_eq!(core.place(request(1, 1, 0)).err(), Some(Fail::Denied));
    let address = state::bar_address();
    core.disable(1, 1).unwrap();
    core.release(1, 1).unwrap();
    assert_eq!(state::releases(), [state::DERIVED]);
    let second = core.place(request(2, 1, 0)).unwrap();
    assert_eq!(second.address, address);
    assert_ne!(second.bar, PieToken::NONE);
    assert_eq!(state::derives(), 2);
    assert!(state::unseals().is_empty());
    core.enable(2, 1).unwrap();
}

#[test]
fn exact_full_aperture_uses_one_derived_heir_and_cached_rebind() {
    state::reset();
    let mut core = full_core();
    let first = core.place(request(1, 1, 0)).unwrap();
    let (address, pages, bar) = (first.address, first.pages, first.bar);
    assert_eq!((address, pages), (0x4_0000_0000_0000, 4));
    assert_eq!(bar, state::REMOTE);
    assert_eq!(state::derives(), 1);
    assert!(state::unseals().is_empty(), "the full aperture is not a sub-Pole child");
    assert!(state::references().is_empty());
    let (source, permission, mark, range) = state::derived();
    assert_eq!(source, state::APERTURE);
    assert_eq!(permission,
        crate::Permission::FETCH | crate::Permission::STORE
            | crate::Permission::VEST | crate::Permission::ONLY);
    assert_eq!(mark, crate::pci_api::BAR_MARK);
    assert_eq!(range, crate::GrantRange::Pages { offset: 0, count: 4 });
    assert_eq!(state::accords().len(), 1);
    assert_eq!(state::accords()[0].0, state::DERIVED);

    let replay = core.place(request(1, 1, 0)).unwrap();
    assert_eq!(replay.bar, bar);
    assert_eq!(state::derives(), 1);
    assert_eq!(state::accords().len(), 1);
    let second_heir = state::derive(
        state::APERTURE, permission, crate::pci_api::BAR_MARK,
        crate::GrantRange::Pages { offset: 0, count: 4 },
    );
    assert_eq!(second_heir.err().map(|error| error.source), Some(crate::PieFail::HandedOver),
        "the full branch keeps exactly one ONLY heir");

    core.enable(1, 1).unwrap();
    core.disable(1, 1).unwrap();
    core.release(1, 1).unwrap();
    assert!(state::releases().is_empty(), "published full root is retained for rebind");

    let rebind = core.place(request(2, 1, 0)).unwrap();
    assert_eq!(rebind.bar, state::REMOTE);
    assert_eq!(state::derives(), 1);
    assert_eq!(state::accords().len(), 1, "cached full root needs no new grant");
    let (before_derives, before_accords) = (state::derives(), state::accords().len());
    assert_eq!(core.place(request(2, 1, 1)).err(), Some(Fail::Denied));
    assert_eq!(state::derives(), before_derives);
    assert_eq!(state::accords().len(), before_accords);
    assert!(state::unseals().is_empty());
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
fn native_geometry_two_functions_bar_four_place_after_first_is_active() {
    state::reset();
    let mut core = core();
    let first = core.place(request(7, 0, 4)).unwrap();
    let (first_address, first_pages, first_bar) = (first.address, first.pages, first.bar);
    assert_eq!(first_address, 0x4_0000_0000_0000);
    assert_eq!(first_pages, 4);
    core.enable(7, 0).unwrap();

    let second = core.place(request(7, 1, 4)).unwrap();
    assert_eq!(second.address, 0x4_0000_0000_4000);
    assert_ne!(second.bar, first_bar);
    assert_eq!(state::derives(), 0);
    let roots = state::unseals();
    assert_eq!(roots.len(), 2);
    assert_eq!((roots[0].offset, roots[0].count), (0, 4));
    assert_eq!((roots[1].offset, roots[1].count), (4, 4));
    assert_ne!(roots[0].token, roots[1].token);
    for root in &roots {
        assert_eq!(
            root.permission,
            crate::Permission::FETCH | crate::Permission::STORE
                | crate::Permission::VEST | crate::Permission::ONLY
        );
    }
    core.enable(7, 1).unwrap();
}

#[test]
fn two_functions_have_distinct_grants_and_independent_master_and_serial_floors() {
    state::reset();
    let mut core = core();
    let first = core.place(request(7, 0, 0)).unwrap();
    let (address, pages, grant) = (first.address, first.pages, first.bar);
    core.enable(7, 0).unwrap();
    assert_eq!(
        ecam::Ecam::new(0x3000_0000, 0).read16(ecam::COMMAND) & ecam::COMMAND_MASTER,
        ecam::COMMAND_MASTER,
        "the first function is active before the second BAR is placed"
    );

    let second = core.place(request(7, 1, 0)).unwrap();
    assert!(second.address >= address + u64::from(pages) * PAGE_SIZE as u64);
    assert_ne!(second.bar, grant);
    assert_eq!(state::derives(), 0, "no BAR consumes the aperture's single ONLY heir");
    let roots = state::unseals();
    assert_eq!(roots.len(), 2);
    assert_ne!(roots[0].token, roots[1].token, "each function owns a distinct root");
    assert_eq!((roots[0].offset, roots[0].count), (0, 4));
    assert_eq!((roots[1].offset, roots[1].count), (4, 4));
    assert_eq!(roots[0].offset + roots[0].count, roots[1].offset, "roots are disjoint");
    for root in &roots {
        assert_eq!(
            root.permission,
            crate::Permission::FETCH | crate::Permission::STORE
                | crate::Permission::VEST | crate::Permission::ONLY
        );
        assert_eq!(root.mark, crate::Mark::NONE);
        assert!(!root.shared);
    }
    assert_eq!(state::reference_releases().len(), 2);

    core.enable(7, 1).unwrap();
    core.disable(7, 0).unwrap();
    assert_eq!(crate::ecam::Ecam::new(0x3000_0000, 1).read16(crate::ecam::COMMAND)
        & crate::ecam::COMMAND_MASTER, crate::ecam::COMMAND_MASTER);
    core.release(7, 0).unwrap();
    assert_eq!(core.place(request(7, 0, 0)).err(), Some(Fail::Conflict));
    core.place(request(8, 0, 0)).unwrap();
    assert_eq!(state::unseals().len(), 2, "published roots are reused per function");
    core.release(7, 0).unwrap();
    core.enable(8, 0).unwrap();
    core.enable(7, 1).unwrap();
    assert_eq!(state::derives(), 0);
}
