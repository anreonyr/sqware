use alloc::vec;
use system_api::operator::{Fail, GrantId, GrantState};

use crate::fixtures::system::System;
use crate::fixtures::{entry, receipt};

#[test]
fn retires_the_original_grant_through_the_stable_root() {
    let root = entry(1);
    let old_anchor = entry(2);
    let id = GrantId(7);
    let system = System::rooted(root);
    let mut installed = receipt(old_anchor, id);

    assert_eq!(system.retire(&mut installed), Ok(true));
    assert_eq!(*system.capability.revoked.borrow(), vec![(root, id)]);
    assert!(installed.grant.is_none());
    assert!(installed.authority.is_none());
    assert!(installed.bindings.is_empty());
    assert!(installed.entries.is_empty());
}

#[test]
fn denied_revoke_keeps_the_whole_receipt() {
    let root = entry(1);
    let id = GrantId(9);
    let system = System::rooted(root);
    *system.capability.revoke_states.borrow_mut() = vec![Err(Fail::Denied)];
    let mut installed = receipt(root, id);

    assert_eq!(system.retire(&mut installed), Err(Fail::Denied));
    assert!(installed.authority.is_some());
    assert!(installed.grant.is_some());
    assert_eq!(installed.bindings.len(), 1);
    assert_eq!(installed.entries.len(), 2);
    assert!(system.capability.unbound.borrow().is_empty());
    assert!(system.capability.unparted.borrow().is_empty());
}

#[test]
fn revoking_keeps_the_receipt_until_the_grant_is_closed() {
    let root = entry(1);
    let id = GrantId(11);
    let system = System::rooted(root);
    *system.capability.revoke_states.borrow_mut() = vec![
        Ok(GrantState::Revoking),
        Ok(GrantState::Closed),
    ];
    let mut installed = receipt(root, id);

    assert_eq!(system.retire(&mut installed), Err(Fail::Dead));
    assert!(installed.authority.is_some());
    assert!(installed.grant.is_some());
    assert_eq!(installed.bindings.len(), 1);
    assert_eq!(installed.entries.len(), 2);
    assert!(system.capability.unbound.borrow().is_empty());
    assert!(system.capability.unparted.borrow().is_empty());

    assert_eq!(system.retire(&mut installed), Ok(true));
    assert!(installed.authority.is_none());
    assert!(installed.grant.is_none());
    assert!(installed.bindings.is_empty());
    assert!(installed.entries.is_empty());
    assert_eq!(system.capability.revoked.borrow().len(), 2);
}

#[test]
fn unpart_unknown_keeps_the_missing_directory_semantics() {
    let root = entry(1);
    let id = GrantId(13);
    let system = System::rooted(root);
    *system.capability.unpart_states.borrow_mut() = vec![Err(Fail::Unknown), Err(Fail::Unknown)];
    let mut installed = receipt(root, id);

    assert_eq!(system.retire(&mut installed), Ok(true));
    assert!(installed.entries.is_empty());
    assert_eq!(
        *system.capability.unparted.borrow(),
        vec![entry(42), entry(41)]
    );
}
