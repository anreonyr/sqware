#![allow(dead_code)]
extern crate alloc;

#[path = "../../src/system/client/src/operator/capability.rs"]
mod capability;

#[cfg(test)]
mod tests {
    use super::capability::{at, coordinates, encode_scope, revoke_unaccepted, valid_descriptor};
    use env::TaskId;
    use system_api::operator::entry;
    use system_api::operator::{
        Entry, EntryId, EntryTarget, GrantId, OperationId, OperationState, Ops, PathBuf, Scope,
    };
    use wire::Message;

    fn anchor() -> Entry {
        Entry {
            operator: TaskId::new(8),
            target: EntryTarget::Node(EntryId::new(12)),
        }
    }

    #[test]
    fn descriptor_metadata_matches_the_system_issuer_not_the_receiving_unit() {
        let issuer = TaskId::new(4);
        let unit = TaskId::new(9);
        let mark = env::Mark::new(77);
        let facts = env::PieInfo {
            token: env::PieToken::mint(12),
            kind: env::PieKind::Hole,
            permission: env::Permission::FETCH,
            owner: issuer,
            vestor: issuer,
            mark,
            alive: true,
        };
        assert!(valid_descriptor(&facts, issuer, mark));
        assert!(!valid_descriptor(&facts, unit, mark));
        assert!(!valid_descriptor(
            &env::PieInfo {
                owner: unit,
                ..facts
            },
            issuer,
            mark
        ));
        assert!(!valid_descriptor(
            &env::PieInfo {
                permission: env::Permission::STORE,
                ..facts
            },
            issuer,
            mark
        ));
    }

    #[test]
    fn bind_loan_is_revoked_before_acceptance_but_retained_after_send() {
        let fail = system_api::operator::Fail::Unknown;
        let operator = TaskId::new(8);
        let imported = env::PieToken::mint(22);
        let source = env::PieToken::mint(11);
        let existing_binding = env::PieToken::mint(33);
        let mut live = alloc::vec![
            (TaskId::new(0), source),
            (operator, imported),
            (operator, existing_binding)
        ];
        let mut targeted = None;
        let mut deferred = Vec::new();
        assert_eq!(
            revoke_unaccepted::<(), _>(
                Err((fail, false)),
                operator,
                imported,
                |target, seed| {
                    targeted = Some((target, seed));
                    let Some(index) = live.iter().position(|item| *item == (target, seed)) else {
                        return Err(env::PieFail::Denied);
                    };
                    live.swap_remove(index);
                    Ok::<(), env::PieFail>(())
                },
                |seed| deferred.push(seed),
            ),
            Err(fail)
        );
        assert_eq!(targeted, Some((operator, imported)));
        assert_eq!(
            live,
            alloc::vec![(TaskId::new(0), source), (operator, existing_binding)]
        );
        assert!(deferred.is_empty());

        let mut revoked = false;
        assert_eq!(
            revoke_unaccepted::<(), _>(
                Err((fail, true)),
                operator,
                imported,
                |_, _| {
                    revoked = true;
                    Ok::<(), env::PieFail>(())
                },
                |_| panic!("an accepted import must remain with the operator"),
            ),
            Err(fail)
        );
        assert!(!revoked);

        let mut deferred = Vec::new();
        assert_eq!(
            revoke_unaccepted::<(), _>(
                Err((fail, false)),
                operator,
                imported,
                |_, _| Err(env::PieFail::Busy),
                |seed| deferred.push(seed),
            ),
            Err(fail)
        );
        assert_eq!(deferred, alloc::vec![imported]);
    }

    #[test]
    fn grant_scope_wire_form_is_bounded_and_preserves_relative_names() {
        assert_eq!(encode_scope(&Scope::Subtree).as_deref(), Some("*"));
        assert_eq!(
            encode_scope(&Scope::Names(alloc::vec![
                PathBuf::try_new("svc").unwrap(),
                PathBuf::try_new("svc/driver").unwrap()
            ]))
            .as_deref(),
            Some("=3:svc10:svc/driver")
        );
        assert_eq!(encode_scope(&Scope::Names(Vec::new())), None);
        let names = alloc::vec![
            PathBuf::try_new(&"a".repeat(100)).unwrap(),
            PathBuf::try_new(&"b".repeat(100)).unwrap(),
            PathBuf::try_new(&"c".repeat(100)).unwrap()
        ];
        assert_eq!(encode_scope(&Scope::Names(names)), None);
    }

    #[test]
    fn find_read_and_revoke_are_distinct_native_authority_requests() {
        let find = at(entry::FIND, anchor(), 0);
        assert_eq!(find.op, entry::FIND);
        assert_eq!(find.anchor, EntryId::new(12));
        let revoke = at(entry::REVOKE, anchor(), GrantId(7).0);
        assert_eq!(revoke.op, entry::REVOKE);
        assert_eq!(revoke.id, 7);
        let read = at(entry::READ, anchor(), 0);
        assert_eq!(read.op, entry::READ);
        assert_eq!(read.anchor, EntryId::new(12));
        assert_eq!(read.root, 0);
        assert_eq!(Ops::ALL.bits() & Ops::FIND.bits(), Ops::FIND.bits());
        let mut bytes = entry::Request::EMPTY;
        let len = find.store(&mut bytes).unwrap();
        assert_eq!(entry::Request::fetch(&bytes[..len]), Some(find));
        let mut bytes = entry::Request::EMPTY;
        let len = read.store(&mut bytes).unwrap();
        assert_eq!(entry::Request::fetch(&bytes[..len]), Some(read));
    }

    #[test]
    fn entry_coordinates_retain_operator_identity() {
        assert_eq!(
            coordinates(TaskId::new(8), anchor()),
            Ok((EntryId::new(12), 0))
        );
        let same_slot_other_operator = Entry {
            operator: TaskId::new(9),
            target: anchor().target,
        };
        assert!(matches!(
            coordinates(TaskId::new(8), same_slot_other_operator),
            Err(system_api::operator::Fail::Denied)
        ));
        let root = Entry {
            operator: TaskId::new(8),
            target: EntryTarget::Root,
        };
        assert_eq!(coordinates(TaskId::new(8), root), Ok((EntryId::new(0), 1)));
    }

    #[test]
    fn mutation_receipt_inspect_and_cancel_keep_the_actual_operation_id() {
        let operation = OperationId(41);
        for op in [entry::INSPECT_OPERATION, entry::CANCEL_OPERATION] {
            let request = at(op, anchor(), operation.0);
            assert_eq!(request.op, op);
            assert_eq!(request.id, operation.0);
            assert_eq!(request.anchor, EntryId::new(12));
            let mut bytes = entry::Request::EMPTY;
            let len = request.store(&mut bytes).unwrap();
            assert_eq!(entry::Request::fetch(&bytes[..len]), Some(request));
        }
        for (wire, expected) in [
            (0, OperationState::Unseen),
            (1, OperationState::Pending),
            (2, OperationState::Applied),
            (3, OperationState::Closed),
            (4, OperationState::Failed),
        ] {
            let response = entry::Response {
                status: system_api::operator::OK,
                state: wire,
                root: 0,
                entry: EntryId::new(12),
                revision: 9,
                grant: 0,
                token: env::PieToken::NONE,
                view: 0,
                permit: system_api::operator::Permit::Public,
            };
            assert_eq!(response.operation_state(), Ok(expected));
        }
    }
}
