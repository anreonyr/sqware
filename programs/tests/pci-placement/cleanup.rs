use crate::pci_api::{self, Fail};
use crate::pci_core::{Core, PlaceRequest};
use crate::platform::Window;
use crate::{GrantRange, PAGE_SIZE, Permission, PieToken, TaskId, state};

const APERTURE_BASE: u64 = 0x4_0000_0000_0000;

fn core() -> Core {
    Core::new(
        0x3000_0000,
        2,
        Window {
            token: state::APERTURE,
            base: APERTURE_BASE,
            size: 4 * 0x4000,
        },
    )
    .unwrap()
}

fn request(serial: u64) -> PlaceRequest {
    PlaceRequest {
        serial,
        requester: 1,
        recipient: TaskId::new(9),
        bar: 0,
    }
}

#[test]
fn failed_delivery_reuses_the_same_exact_bar_derivation_and_handle() {
    state::reset();
    state::fail_next_accord();
    let mut core = core();
    assert_eq!(core.place(request(1)).err(), Some(Fail::Denied));
    assert!(state::releases().is_empty(), "failure must retain the local root");

    assert_eq!(state::derives(), 1);
    let (source, permission, mark, range) = state::derived();
    assert_eq!(source, state::APERTURE);
    assert_eq!(
        permission,
        Permission::FETCH
            | Permission::STORE
            | Permission::VEST
            | Permission::ONLY
    );
    assert_eq!(mark, pci_api::BAR_MARK);
    assert_eq!(
        range,
        GrantRange::Pages {
            offset: 0,
            count: state::BAR_SIZE as usize / PAGE_SIZE,
        }
    );
    assert_eq!(state::bar_address(), APERTURE_BASE);
    let first = state::accords();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].0, state::DERIVED);
    assert_eq!(first[0].1, TaskId::new(9));
    assert_eq!(first[0].2, permission);
    assert_eq!(first[0].3, pci_api::BAR_MARK);

    let placement = core.place(request(1)).unwrap();
    assert_eq!(placement.bar, state::REMOTE);
    assert_eq!(state::derives(), 1, "retry must not derive a second time");
    let second = state::accords();
    assert_eq!(second.len(), 2);
    assert_eq!(second[1].0, first[0].0, "retry must reuse the same local root");
    assert_eq!(second[1].1, first[0].1);
    assert_eq!(second[1].2, first[0].2);
    assert_eq!(second[1].3, first[0].3);

    let again = core.place(request(1)).unwrap();
    assert_eq!(again.bar, state::REMOTE);
    assert_eq!(state::derives(), 1);
    assert_eq!(state::accords().len(), 2, "successful replay is idempotent");
}

#[test]
fn failed_delivery_holds_the_local_root_until_cleanup() {
    state::reset();
    state::fail_next_accord();
    let mut core = core();
    assert_eq!(core.place(request(1)).err(), Some(Fail::Denied));
    assert!(state::releases().is_empty(), "failure must retain the root");
    drop(core);
    assert_eq!(state::releases(), vec![state::DERIVED]);
}

#[test]
fn a_different_binding_cannot_take_over_a_pending_delivery() {
    state::reset();
    state::fail_next_accord();
    let mut core = core();
    assert_eq!(core.place(request(1)).err(), Some(Fail::Denied));
    assert_eq!(core.place(request(2)).err(), Some(Fail::Conflict));
    assert_eq!(state::derives(), 1);
}

#[test]
fn cancelled_delivery_retries_its_cleanup_before_allowing_a_new_binding() {
    state::reset();
    state::fail_next_accord();
    let mut core = core();
    assert_eq!(core.place(request(1)).err(), Some(Fail::Denied));
    assert_eq!(core.release(1, 1), Err(Fail::Conflict));
    assert!(state::releases().is_empty());
    assert_eq!(core.disable(2, 1), Err(Fail::Conflict));
    core.disable(1, 1).unwrap();
    state::fail_next_release();
    assert_eq!(core.release(1, 1), Err(Fail::Denied));
    assert_eq!(core.place(request(2)).err(), Some(Fail::Conflict));
    assert_eq!(state::derives(), 1);
    core.release(1, 1).unwrap();
    assert_eq!(state::releases(), vec![state::DERIVED, state::DERIVED]);
    assert_eq!(core.place(request(1)).err(), Some(Fail::Conflict));
    assert_eq!(core.place(request(2)).unwrap().bar, state::REMOTE);
    assert_eq!(state::derives(), 2);
    core.release(1, 1).unwrap();
    assert_eq!(state::releases(), vec![state::DERIVED, state::DERIVED]);
    core.enable(2, 1).unwrap();
}

#[test]
fn exact_bar_root_is_a_child_of_the_wide_aperture() {
    state::reset();
    let mut core = core();
    let placement = core.place(request(1)).unwrap();
    assert_eq!(placement.pages, 4);
    assert_eq!(placement.size, state::BAR_SIZE);
    let (source, _, _, _) = state::derived();
    assert_eq!(source, state::APERTURE);
    assert_ne!(state::DERIVED, state::APERTURE);
    assert_eq!(state::accords()[0].0, state::DERIVED);
    assert!(state::releases().is_empty(), "success must retain the local root");
    assert_eq!(state::revokes(), Vec::<(TaskId, PieToken)>::new());
}

#[test]
fn invalid_requester_is_rejected_before_derivation() {
    state::reset();
    let mut core = core();
    let mut bad = request(1);
    bad.requester = 2;
    assert_eq!(core.place(bad).err(), Some(Fail::Denied));
    assert_eq!(state::derives(), 0);
    assert!(state::accords().is_empty());
}
