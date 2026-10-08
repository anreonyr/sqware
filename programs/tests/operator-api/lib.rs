#![allow(dead_code)]

pub mod system {
    pub mod identity {
        pub use system_api::identity::*;
    }
}
#[path = "../../src/system/operator/tree/gate.rs"]
mod gate;
#[path = "../../src/system/operator/tree/judge.rs"]
mod judge;

#[path = "../../src/system/operator/service/admission.rs"]
mod admission;

#[cfg(test)]
mod tests {
    use env::{PieToken, TaskId};
    use system_api::identity::{PrincipalId, Selector};
    use system_api::operator::*;
    use wire::{Field, Message, Span};

    #[test]
    fn native_caller_controls_mutation_on_the_unified_session() {
        let control = TaskId::new(7);
        let other = TaskId::new(8);
        let requests = [
            Req::Part {
                at: Where::Root,
                name: "new".into(),
            },
            Req::Land {
                at: Where::Root,
                name: "new".into(),
                entry: PieToken::mint(9),
                permit: Permit::Public,
                mine: false,
            },
            Req::Trim(EntryId::new(0)),
            Req::Find(EntryId::new(0)),
            Req::List(Where::Root),
            Req::Road(Path::new("svc").to_path_buf()),
            Req::Name(EntryId::new(0)),
            Req::Watch {
                road: Path::new("svc").to_path_buf(),
                hole: PieToken::mint(10),
            },
        ];
        for (index, request) in requests.iter().enumerate() {
            let mut bytes = Req::EMPTY;
            let n = request.store(&mut bytes).unwrap();
            let decoded = Req::fetch(&bytes[..n]).unwrap();
            assert!(super::admission::caller(control, control, &decoded));
            assert_eq!(
                super::admission::caller(other, control, &decoded),
                index >= 3
            );
        }
    }

    fn word(bytes: &mut Vec<u8>, value: u64) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    #[test]
    fn guest_handoff_carries_an_explicit_recipient_token() {
        let request = Tip::Guest {
            who: TaskId::new(7),
            reply: PieToken::mint(9),
        };
        let mut expected = vec![2];
        word(&mut expected, 7);
        word(&mut expected, 9);
        let mut bytes = vec![0; TIP_LEN];
        let used = request.store(&mut bytes).unwrap();
        assert_eq!(used, 17);
        assert_eq!(&bytes[..used], expected.as_slice());
        assert!(
            matches!(TipIn::fetch(&expected), Some(TipIn::Guest { who, reply })
            if who == TaskId::new(7) && reply == PieToken::mint(9))
        );
        assert!(
            TipIn::fetch(&expected[..9]).is_none(),
            "implicit legacy handoff must not rescan"
        );
        expected.push(0);
        assert!(TipIn::fetch(&expected).is_none());
    }

    #[test]
    fn request_goldens_preserve_action_codes_and_distinct_grant_order() {
        let road = Path::new("svc").to_path_buf();
        let root = vec![0; 9];
        let mut part = vec![2];
        part.extend_from_slice(&root);
        part.extend_from_slice(&[1, b'x']);
        let mut land = vec![1];
        land.extend_from_slice(&root);
        land.extend_from_slice(&[1, b'x']);
        word(&mut land, 9);
        land.push(1);
        land.extend_from_slice(&[0; 18]);
        let mut watch = vec![8, 3, b's', b'v', b'c'];
        word(&mut watch, 9);
        let mut list = vec![5];
        list.extend_from_slice(&root);
        let cases = [
            (
                Req::Part {
                    at: Where::Root,
                    name: "x".into(),
                },
                Wire::Part {
                    at: Where::Root,
                    name: "x".into(),
                },
                part,
                Grant::Part,
            ),
            (
                Req::Land {
                    at: Where::Root,
                    name: "x".into(),
                    entry: PieToken::mint(9),
                    permit: Permit::Public,
                    mine: true,
                },
                Wire::Land {
                    at: Where::Root,
                    name: "x".into(),
                    entry: PieToken::mint(9),
                    permit: Permit::Public,
                    mine: true,
                },
                land,
                Grant::Land,
            ),
            (
                Req::Find(EntryId::new(0)),
                Wire::Find(EntryId::new(0)),
                vec![3, 0, 0, 0, 0, 0, 0, 0, 0],
                Grant::Find,
            ),
            (
                Req::Trim(EntryId::new(0)),
                Wire::Trim(EntryId::new(0)),
                vec![4, 0, 0, 0, 0, 0, 0, 0, 0],
                Grant::Trim,
            ),
            (
                Req::List(Where::Root),
                Wire::List(Where::Root),
                list,
                Grant::List,
            ),
            (
                Req::Road(road.clone()),
                Wire::Road(road.clone()),
                vec![7, 3, b's', b'v', b'c'],
                Grant::Seek,
            ),
            (
                Req::Name(EntryId::new(0)),
                Wire::Name(EntryId::new(0)),
                vec![6, 0, 0, 0, 0, 0, 0, 0, 0],
                Grant::Name,
            ),
            (
                Req::Watch {
                    road: road.clone(),
                    hole: PieToken::mint(9),
                },
                Wire::Watch {
                    road,
                    hole: PieToken::mint(9),
                },
                watch,
                Grant::Watch,
            ),
        ];
        for (index, (request, decoded, expected, grant)) in cases.into_iter().enumerate() {
            let mut buffer = Req::EMPTY;
            let len = request.store(&mut buffer).unwrap();
            assert_eq!(&buffer[..len], expected);
            assert_eq!(Req::fetch(&expected), Some(decoded.clone()));
            assert_eq!(Grant::for_wire(&decoded), grant);
            assert_eq!(grant.at(), index as u8 + 1);
            assert_eq!(Grant::from_action(grant.at()), Some(grant));
            assert_eq!(
                grant.mark(),
                env::Mark::of(&format!("operator-ask-{}", grant.name()))
            );
            let mut tailed = expected.clone();
            tailed.push(0);
            assert_eq!(Req::fetch(&tailed), None);
            for end in 0..expected.len() {
                assert_eq!(Req::fetch(&expected[..end]), None);
            }
        }
    }

    #[test]
    fn reply_goldens_keep_entry_seed_list_and_name_shapes() {
        for (reply, expected) in [
            (Union::Status(DENIED), vec![8]),
            (Union::Entry(EntryId::new(0)), vec![0; 9]),
            (
                Union::Seed(PieToken::mint(9)),
                vec![0, 9, 0, 0, 0, 0, 0, 0, 0],
            ),
            (Union::Name("x".into()), vec![0, b'x']),
            (
                Union::List(Listing::of([EntryId::new(0), EntryId::new(9)].into_iter())),
                vec![0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 9, 0, 0, 0, 0, 0, 0, 0],
            ),
        ] {
            let mut buffer = Union::EMPTY;
            let len = reply.store(&mut buffer).unwrap();
            assert_eq!(&buffer[..len], expected);
            let received = Union::fetch(&expected).unwrap();
            match reply {
                Union::Status(code) => assert_eq!(received.code(), code),
                Union::Entry(id) => assert_eq!(received.entry(), Ok(id)),
                Union::Seed(seed) => assert_eq!(received.seed(), Ok(seed)),
                Union::Name(name) => assert_eq!(received.name(), Ok(name)),
                Union::List(ids) => assert_eq!(received.list(), Ok(ids)),
            }
        }
        assert_eq!(Union::fetch(&[]), None);
        let malformed = Union::fetch(&[0, 2, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(malformed.list(), Err(BAD));
    }

    #[test]
    fn permits_keep_authority_and_reject_unused_payloads() {
        let selector = Selector::Exact(PrincipalId::new(TaskId::new(5), 0));
        let mut buffer = [0; 18];
        Permit::Identity(selector).store(&mut buffer);
        let mut expected = vec![2, 0];
        word(&mut expected, 5);
        word(&mut expected, 0);
        assert_eq!(buffer.as_slice(), expected);
        assert_eq!(Permit::fetch(&buffer), Some(Permit::Identity(selector)));
        for permit in [
            Permit::Public,
            Permit::Bound,
            Permit::Opener(EntryId::new(0)),
        ] {
            permit.store(&mut buffer);
            assert_eq!(Permit::fetch(&buffer), Some(permit));
            buffer[17] = 1;
            assert_eq!(Permit::fetch(&buffer), None);
        }
    }

    #[test]
    fn bootstrap_and_event_goldens_keep_separate_coordinates() {
        let mut buffer = vec![0; TIP_LEN];
        let tip = Tip::Wired {
            authority: TaskId::new(5),
            resolve: PieToken::mint(1),
            matches: PieToken::mint(2),
            same: PieToken::mint(3),
            back: PieToken::mint(9),
        };
        let len = tip.store(&mut buffer).unwrap();
        let mut expected = vec![3];
        for value in [5, 1, 2, 3, 9] {
            word(&mut expected, value);
        }
        assert_eq!(&buffer[..len], expected);
        assert!(
            matches!(TipIn::fetch(&expected), Some(TipIn::Wired { authority, back, .. }) if authority == TaskId::new(5) && back == PieToken::mint(9))
        );
        let event = Event {
            seq: 1,
            kind: Kind::Landed,
            road: Path::new("svc").to_path_buf(),
            id: EntryId::new(0),
            owner: TaskId::new(5),
        };
        let len = event.store(&mut buffer).unwrap();
        let mut expected = Vec::new();
        word(&mut expected, 1);
        expected.extend_from_slice(&[1, 3, b's', b'v', b'c']);
        word(&mut expected, 0);
        word(&mut expected, 5);
        assert_eq!(&buffer[..len], expected);
        assert_eq!(Event::fetch(&expected), Some(event));
    }

    #[test]
    fn path_bounds_and_canonical_form_survive_the_facade_move() {
        assert_eq!(Path::new("/svc/sys/operator").as_str(), "svc/sys/operator");
        assert_eq!(PathBuf::try_new("svc//sys").unwrap().as_str(), "svc/sys");
        assert_eq!(PathBuf::try_new("a/b/c/d/e/f/g/h/i"), None);
        let road = Path::new("svc/sys").to_path_buf();
        let mut buffer = [0; Path::LEN];
        let len = road.store_at(&mut buffer, 0).unwrap();
        assert_eq!(&buffer[..len], b"\x07svc/sys");
        assert_eq!(PathBuf::fetch_at(&buffer[..len], 0).unwrap().0, road);
    }

    #[test]
    fn marks_and_gate_codes_stay_fixed() {
        for (mark, name) in [
            (ASK_MARK, "operator-ask"),
            (TIP_MARK, "tip"),
            (TIP_BACK, "operator-tip-back"),
            (LINK_MARK, "operator"),
            (WATCH_MARK, "operator-watch"),
        ] {
            assert_eq!(mark, env::Mark::of(name));
        }
        assert_eq!(env::marks::conflict(REGISTRY), None);
        assert_eq!(super::gate::Code::Ok.wire(), OK);
        assert_eq!(super::gate::Code::Denied.wire(), DENIED);
        assert_eq!(super::gate::Code::Unjudged.wire(), UNJUDGED);
    }

    #[test]
    fn sdk_uses_one_face_without_grant_selected_handles() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../src/system/client/src/operator");
        let client = std::fs::read_to_string(root.join("client/mod.rs")).unwrap();
        let module = std::fs::read_to_string(root.join("mod.rs")).unwrap();
        for method in [
            "part", "land", "find", "trim", "list", "seek", "name", "watch",
        ] {
            assert!(
                client.contains(&format!("pub fn {method}(")),
                "Face::{method} missing"
            );
        }
        for source in [&client, &module] {
            assert!(!source.contains("Rein"));
            assert!(!source.contains("granted_berth"));
            assert!(!source.contains("fn rein("));
        }
    }
}
