#![allow(dead_code)]
extern crate alloc;

#[path = "../../src/system/identity/export_admission.rs"]
mod export_admission;

#[test]
fn identity_export_retry_requires_matching_program_and_delivered_receipt() {
    use export_admission::ExistingGrant::{Conflict, Delivered, Retry};

    assert_eq!(export_admission::existing_grant(true, true), Delivered);
    assert_eq!(export_admission::existing_grant(true, false), Retry);
    assert_eq!(export_admission::existing_grant(false, true), Conflict);
    assert_eq!(export_admission::existing_grant(false, false), Conflict);
}

#[test]
fn identity_export_uses_authoritative_operator_entry_reference() {
    use env::{PieToken, TaskId};
    use system_api::identity::export::{ENTRY, Request};
    use system_api::identity::{IdentityName, IdentityObject, OperatorEntryRef, PrincipalId};
    use wire::Message;
    let object = IdentityObject::Principal(PrincipalId::new(TaskId::new(3), 19));
    let alias = IdentityName::new("account-name").unwrap();
    let binding = IdentityName::new("Account").unwrap();
    let entry = OperatorEntryRef {
        operator: TaskId::new(21),
        root: false,
        id: 0,
        revision: 47,
    };
    let request = Request {
        object,
        alias,
        binding,
        entry,
        back: PieToken::NONE,
    };
    let mut bytes = [0; Request::LEN];
    let used = request.store(&mut bytes).unwrap();
    let (decoded, exact) = Request::fetch(&bytes[..used]).unwrap();
    assert!(exact && decoded.valid());
    assert_eq!(decoded.object, object);
    assert_eq!(decoded.alias, alias);
    assert_eq!(decoded.binding, binding);
    assert_eq!(decoded.entry, entry);
    assert_eq!(ENTRY, system_api::identity::export::ENTRY);

    let node_zero = OperatorEntryRef {
        root: false,
        id: 0,
        ..entry
    };
    assert!(
        Request {
            entry: node_zero,
            ..request
        }
        .valid()
    );
    assert!(
        !Request {
            entry: OperatorEntryRef {
                root: true,
                id: 1,
                ..entry
            },
            ..request
        }
        .valid()
    );
    assert!(
        !Request {
            entry: OperatorEntryRef {
                operator: TaskId::new(0),
                ..entry
            },
            ..request
        }
        .valid()
    );
}

#[test]
fn identity_export_admission_uses_registered_source_and_roster_subtree() {
    use env::{PieToken, TaskId};
    use system_api::identity::{
        IdentityName, IdentityObject, OperatorEntryRef, PrincipalId, Subject,
    };
    let request = system_api::identity::export::Request {
        object: IdentityObject::Principal(PrincipalId::new(TaskId::new(3), 19)),
        alias: IdentityName::new("account-name").unwrap(),
        binding: IdentityName::new("Account").unwrap(),
        entry: OperatorEntryRef {
            operator: TaskId::new(21),
            root: false,
            id: 0,
            revision: 47,
        },
        back: PieToken::mint(91),
    };
    let mut calls = Vec::new();
    let result = export_admission::authorize_alias(
        export_admission::CallerFacts {
            from: TaskId::new(7),
            owner: TaskId::new(7),
            live: true,
            named: true,
        },
        export_admission::AliasClaim {
            aliases: &["account-name"],
            request: &request,
        },
        |from, subject| {
            calls.push((from, subject));
            Ok(())
        },
    );
    assert_eq!(
        result,
        Ok(Subject::new(PrincipalId::new(TaskId::new(3), 19), &[]).unwrap())
    );
    assert_eq!(calls, [(TaskId::new(7), result.unwrap())]);

    let denied = export_admission::authorize_alias(
        export_admission::CallerFacts {
            from: TaskId::new(7),
            owner: TaskId::new(7),
            live: true,
            named: true,
        },
        export_admission::AliasClaim {
            aliases: &["account-name"],
            request: &request,
        },
        |_, _| Err(()),
    );
    assert_eq!(denied, Err(export_admission::Failure::Denied));
}

#[test]
fn identity_export_admission_preserves_source_owner_and_forged_reply_guards() {
    use env::{PieToken, TaskId};
    use system_api::identity::{IdentityName, IdentityObject, OperatorEntryRef, PrincipalId};
    let request = system_api::identity::export::Request {
        object: IdentityObject::Principal(PrincipalId::new(TaskId::new(3), 19)),
        alias: IdentityName::new("account-name").unwrap(),
        binding: IdentityName::new("Account").unwrap(),
        entry: OperatorEntryRef {
            operator: TaskId::new(21),
            root: false,
            id: 0,
            revision: 47,
        },
        back: PieToken::mint(91),
    };
    for (caller, aliases) in [
        (
            export_admission::CallerFacts {
                from: TaskId::new(8),
                owner: TaskId::new(7),
                live: true,
                named: true,
            },
            &["account-name"][..],
        ),
        (
            export_admission::CallerFacts {
                from: TaskId::new(7),
                owner: TaskId::new(7),
                live: false,
                named: true,
            },
            &["account-name"][..],
        ),
        (
            export_admission::CallerFacts {
                from: TaskId::new(7),
                owner: TaskId::new(7),
                live: true,
                named: false,
            },
            &["account-name"][..],
        ),
        (
            export_admission::CallerFacts {
                from: TaskId::new(7),
                owner: TaskId::new(7),
                live: true,
                named: true,
            },
            &["other"][..],
        ),
    ] {
        let mut checked_roster = false;
        let result = export_admission::authorize_alias(
            caller,
            export_admission::AliasClaim {
                aliases,
                request: &request,
            },
            |_, _| {
                checked_roster = true;
                Ok(())
            },
        );
        assert_eq!(result, Err(export_admission::Failure::Denied));
        assert!(!checked_roster);
    }

    let trusted_back = request.back;
    let forged = system_api::identity::export::Request {
        back: PieToken::mint(92),
        ..request
    };
    let response_target = system_api::identity::export::reply_to(&(request, true));
    assert_eq!(response_target, trusted_back);
    assert_ne!(
        system_api::identity::export::reply_to(&(forged, true)),
        trusted_back
    );
}

#[test]
fn private_lookup_cleanup_keeps_both_resources_and_deduplicates_shared_tokens() {
    use env::PieToken;
    let located = PieToken::mint(101);
    let actual = PieToken::mint(102);
    let mut released = Vec::new();
    export_admission::keep_lookup_resources(located, Some(actual), |token| {
        released.push(token);
        Ok(())
    })
    .unwrap();
    assert_eq!(released, [located, actual]);

    released.clear();
    export_admission::keep_lookup_resources(located, Some(located), |token| {
        released.push(token);
        Ok(())
    })
    .unwrap();
    assert_eq!(released, [located]);

    released.clear();
    assert!(
        export_admission::keep_lookup_resources(located, None, |token| {
            released.push(token);
            Err(())
        })
        .is_err()
    );
    assert_eq!(released, [located]);
}
