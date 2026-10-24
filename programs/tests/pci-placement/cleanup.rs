use crate::pci_api::{self, Fail};
use crate::pci_core::{Core, PlaceRequest};
use crate::platform::Window;
use crate::{
    AcquireSource, GrantRange, Permission, PieFail, PieToken, TaskId, state,
};

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
fn failed_delivery_reuses_the_same_exact_child_root_and_handle() {
    state::reset();
    state::fail_next_accord();
    let mut core = core();
    assert_eq!(core.place(request(1)).err(), Some(Fail::Denied));
    assert!(state::releases().is_empty(), "failure must retain the local root");
    assert_eq!(state::derives(), 0, "the broad aperture's ONLY heir is not published");

    assert_eq!(state::unseals().len(), 1);
    let unseal = state::unseals()[0];
    assert_eq!(unseal.permission,
        Permission::FETCH | Permission::STORE | Permission::VEST | Permission::ONLY);
    assert_eq!(unseal.mark, crate::Mark::NONE);
    assert_eq!((unseal.offset, unseal.count), (0, 4));
    assert!(!unseal.shared);
    let handle = state::references()[0].handle;
    assert_eq!(unseal.reference, handle);
    assert!(state::references()[0].released, "temporary parent window is closed before delivery");
    assert_eq!(state::reference_releases(), vec![handle]);
    assert_eq!(state::bar_address(), APERTURE_BASE);

    let first = state::accords();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].0, unseal.token);
    assert_eq!(first[0].1, TaskId::new(9));
    assert_eq!(first[0].2, unseal.permission);
    assert_eq!(first[0].3, pci_api::BAR_MARK);

    let placement = core.place(request(1)).unwrap();
    assert_eq!(placement.bar, state::REMOTE);
    assert_eq!(state::unseals().len(), 1, "retry must not partition a second time");
    assert_eq!(state::derives(), 0);
    let second = state::accords();
    assert_eq!(second.len(), 2);
    assert_eq!(second[1].0, first[0].0, "retry must reuse the same local root");
    assert_eq!(second[1].1, first[0].1);
    assert_eq!(second[1].2, first[0].2);
    assert_eq!(second[1].3, first[0].3);

    let again = core.place(request(1)).unwrap();
    assert_eq!(again.bar, state::REMOTE);
    assert_eq!(state::unseals().len(), 1);
    assert_eq!(state::accords().len(), 2, "successful replay is idempotent");
}

#[test]
fn failed_delivery_holds_the_local_root_until_cleanup() {
    state::reset();
    state::fail_next_accord();
    let mut core = core();
    assert_eq!(core.place(request(1)).err(), Some(Fail::Denied));
    assert!(state::releases().is_empty(), "failure must retain the root");
    let child = state::unseals()[0].token;
    drop(core);
    assert_eq!(state::releases(), vec![child]);
}

#[test]
fn a_different_binding_cannot_take_over_a_pending_delivery() {
    state::reset();
    state::fail_next_accord();
    let mut core = core();
    assert_eq!(core.place(request(1)).err(), Some(Fail::Denied));
    assert_eq!(core.place(request(2)).err(), Some(Fail::Conflict));
    assert_eq!(state::unseals().len(), 1);
    assert_eq!(state::derives(), 0);
}

#[test]
fn cancelled_delivery_retries_its_cleanup_before_allowing_a_new_binding() {
    state::reset();
    state::fail_next_accord();
    let mut core = core();
    assert_eq!(core.place(request(1)).err(), Some(Fail::Denied));
    let child = state::unseals()[0].token;
    assert_eq!(core.release(1, 1), Err(Fail::Conflict));
    assert!(state::releases().is_empty());
    assert_eq!(core.disable(2, 1), Err(Fail::Conflict));
    core.disable(1, 1).unwrap();
    state::fail_next_release();
    assert_eq!(core.release(1, 1), Err(Fail::Denied));
    assert_eq!(core.place(request(2)).err(), Some(Fail::Conflict));
    assert_eq!(state::unseals().len(), 1);
    core.release(1, 1).unwrap();
    assert_eq!(state::releases(), vec![child, child]);
    assert_eq!(core.place(request(1)).err(), Some(Fail::Conflict));
    assert_eq!(core.place(request(2)).unwrap().bar, state::REMOTE);
    assert_eq!(state::unseals().len(), 2);
    core.release(1, 1).unwrap();
    assert_eq!(state::releases(), vec![child, child]);
    core.enable(2, 1).unwrap();
}

#[test]
fn exact_bar_root_is_an_independent_child_of_the_wide_aperture() {
    state::reset();
    let mut core = core();
    let placement = core.place(request(1)).unwrap();
    assert_eq!(placement.pages, 4);
    assert_eq!(placement.size, state::BAR_SIZE);
    assert_eq!(state::derives(), 0);
    let unseal = state::unseals()[0];
    assert_eq!(unseal.permission,
        Permission::FETCH | Permission::STORE | Permission::VEST | Permission::ONLY);
    assert_eq!(unseal.mark, crate::Mark::NONE);
    assert_ne!(unseal.token, state::APERTURE);
    assert_eq!(unseal.token, state::accords()[0].0);
    assert_eq!(state::accords()[0].3, pci_api::BAR_MARK);
    assert_eq!(state::references()[0].offset, 0);
    assert_eq!(state::references()[0].count, 4);
    assert!(state::releases().is_empty(), "success must retain the local root");
    assert_eq!(state::revokes(), Vec::<(TaskId, PieToken)>::new());
}

#[test]
fn invalid_requester_is_rejected_before_partitioning() {
    state::reset();
    let mut core = core();
    let mut bad = request(1);
    bad.requester = 2;
    assert_eq!(core.place(bad).err(), Some(Fail::Denied));
    assert_eq!(state::derives(), 0);
    assert!(state::unseals().is_empty());
    assert!(state::references().is_empty());
    assert!(state::accords().is_empty());
}

#[test]
fn failed_unseal_retains_the_exact_reference_and_retries_the_same_window() {
    state::reset();
    state::fail_next_unseal();
    let mut core = core();
    assert_eq!(core.place(request(1)).err(), Some(Fail::Denied));
    assert_eq!(state::unseals().len(), 0);
    assert_eq!(state::references().len(), 1);
    let handle = state::references()[0].handle;
    assert!(!state::references()[0].released);
    assert!(state::reference_releases().is_empty());
    assert_eq!(state::bar_address(), APERTURE_BASE);

    let placement = core.place(request(1)).unwrap();
    assert_eq!(placement.bar, state::REMOTE);
    assert_eq!(state::references().len(), 1, "the same parent reference is retried");
    assert_eq!(state::references()[0].handle, handle);
    assert_eq!(state::unseals().len(), 1);
    assert_eq!(state::reference_releases(), vec![handle]);
}

#[test]
fn failed_parent_release_blocks_grant_then_reuses_the_same_root() {
    state::reset();
    state::fail_next_reference_release();
    let mut core = core();
    assert_eq!(core.place(request(1)).err(), Some(Fail::Denied));
    let child = state::unseals()[0].token;
    let handle = state::references()[0].handle;
    assert!(!state::references()[0].released);
    assert!(state::reference_releases().is_empty());
    assert!(state::accords().is_empty(), "delivery waits for the temporary parent close");

    let placement = core.place(request(1)).unwrap();
    assert_eq!(placement.bar, state::REMOTE);
    assert_eq!(state::unseals().len(), 1);
    assert_eq!(state::unseals()[0].token, child);
    assert_eq!(state::reference_releases(), vec![handle]);
    assert_eq!(state::accords().len(), 1);
    assert_eq!(state::accords()[0].0, child);
}

#[test]
fn unpublished_release_closes_reference_and_root_without_losing_geometry() {
    state::reset();
    state::fail_next_unseal();
    let mut core = core();
    assert_eq!(core.place(request(1)).err(), Some(Fail::Denied));
    let handle = state::references()[0].handle;
    core.disable(1, 1).unwrap();
    core.release(1, 1).unwrap();
    assert_eq!(state::reference_releases(), vec![handle]);
    assert!(state::releases().is_empty());
    assert_eq!(state::bar_address(), APERTURE_BASE);

    let placement = core.place(request(2)).unwrap();
    assert_eq!(placement.address, APERTURE_BASE);
    assert_eq!(state::unseals().len(), 1);
    assert_eq!(state::derives(), 0);
}

#[test]
fn unpublished_release_closes_parent_reference_before_child_root_and_retries_both() {
    state::reset();
    state::fail_next_reference_release();
    let mut core = core();
    assert_eq!(core.place(request(1)).err(), Some(Fail::Denied));
    let child = state::unseals()[0].token;
    let handle = state::references()[0].handle;
    assert_eq!(state::unseals().len(), 1);
    assert!(state::accords().is_empty());
    assert!(!state::references()[0].released);
    assert!(state::reference_releases().is_empty());

    core.disable(1, 1).unwrap();
    state::fail_next_reference_release();
    assert_eq!(core.release(1, 1), Err(Fail::Denied));
    assert_eq!(state::references()[0].handle, handle);
    assert!(!state::references()[0].released);
    assert_eq!(state::unseals()[0].token, child);
    assert!(state::releases().is_empty(), "parent cleanup keeps the child root owned");
    assert!(state::reference_releases().is_empty());

    core.release(1, 1).unwrap();
    assert_eq!(state::reference_releases(), vec![handle]);
    assert_eq!(state::releases(), vec![child]);
    assert_eq!(core.place(request(1)).err(), Some(Fail::Conflict));
    let placement = core.place(request(2)).unwrap();
    assert_eq!(placement.address, APERTURE_BASE);
    assert_eq!(placement.bar, state::REMOTE);
    assert_eq!(state::unseals().len(), 2);
    assert_eq!(state::derives(), 0);
    assert_eq!(state::accords().len(), 1);
    assert_eq!(state::accords()[0].0, state::unseals()[1].token,
        "old serial granted nothing; the rebound root is the one delivered");
    core.enable(2, 1).unwrap();
}

#[test]
fn old_same_only_parent_derive_cannot_produce_two_sibling_heirs() {
    state::reset();
    let only = Permission::FETCH | Permission::STORE | Permission::VEST | Permission::ONLY;
    state::derive(state::APERTURE, only, pci_api::BAR_MARK,
        GrantRange::Pages { offset: 8, count: 4 }).unwrap();
    let second = state::derive(state::APERTURE, only, pci_api::BAR_MARK,
        GrantRange::Pages { offset: 12, count: 4 });
    assert_eq!(second.err().map(|error| error.source), Some(PieFail::HandedOver),
        "one ONLY parent cannot publish two sibling heirs");

    let handle = state::acquire(AcquireSource::Source {
        token: state::APERTURE, offset: 0, count: 4,
    }, PieToken::NONE).unwrap();
    let root = state::unseal(handle, false).unwrap();
    assert_eq!(state::unseals().len(), 1);
    assert_ne!(root, state::APERTURE);
    assert_eq!(state::references()[0].offset, 0);
}
