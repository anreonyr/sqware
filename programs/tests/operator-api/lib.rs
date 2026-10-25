#![allow(dead_code)]
extern crate alloc;

#[cfg(test)]
use unit_declarations::operator as unit_operator;

pub mod system {
    pub mod identity {
        pub use system_api::identity::*;
    }
}
#[path = "../../src/system/operator/authority.rs"]
mod authority;
#[path = "../../src/system/operator/tree/gate.rs"]
mod gate;
#[path = "../../src/system/operator/tree/judge.rs"]
mod judge;

#[cfg(test)]
mod tests {
    use env::{Access, Mark, Permission, PieToken, TaskId};
    use system_api::identity::{PrincipalId, Selector};
    use system_api::operator::*;
    use wire::{Field, Message, Span};

    #[test]
    fn relative_watch_request_preserves_entry_root_tag() {
        let request = Req::WatchFrom {
            base: Entry {
                operator: TaskId::new(7),
                target: EntryTarget::Root,
            },
            road: PathBuf::try_new("control").unwrap(),
            hole: PieToken::mint(9),
        };
        let mut bytes = Req::EMPTY;
        let len = request.store(&mut bytes).unwrap();
        assert_eq!(
            Req::fetch(&bytes[..len]),
            Some(Wire::WatchFrom {
                base: Entry {
                    operator: TaskId::new(7),
                    target: EntryTarget::Root
                },
                road: PathBuf::try_new("control").unwrap(),
                hole: PieToken::mint(9),
            })
        );
    }

    #[test]
    fn former_shared_ask_mutation_codes_are_not_decodable() {
        for code in [1, 2, 4] {
            let mut frame = vec![code];
            frame.extend_from_slice(&[0; 60]);
            assert!(Req::fetch(&frame).is_none(), "legacy code {code} decoded");
        }
    }

    #[test]
    fn former_tip_tree_mutations_are_not_decodable() {
        for code in [1, 4, 5, 6] {
            let mut frame = vec![code];
            frame.extend_from_slice(&[0; TIP_LEN]);
            assert!(
                TipIn::fetch(&frame).is_none(),
                "legacy TIP code {code} decoded"
            );
        }
    }

    #[test]
    fn resource_views_are_nonempty_and_cannot_expand_source_rights() {
        let fetch = ResourceView::new(Access::FETCH).unwrap();
        let store = ResourceView::new(Access::STORE).unwrap();
        assert!(ResourceView::new(Access::NONE).is_none());
        assert_eq!(ResourceView::from_bits(fetch.bits()), Some(fetch));
        assert_eq!(ResourceView::from_bits(1 << 2), None);
        assert!(fetch.fits(Permission::FETCH | Permission::VEST));
        assert!(store.fits(Permission::STORE | Permission::VEST));
        assert!(!store.fits(Permission::FETCH | Permission::VEST));
        assert!(!fetch.fits(Permission::FETCH));
    }

    fn word(bytes: &mut Vec<u8>, value: u64) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    #[test]
    fn guest_handoff_carries_an_explicit_recipient_token() {
        let request = Tip::Guest {
            who: TaskId::new(7),
            reply: PieToken::mint(9),
            ask: PieToken::mint(11),
        };
        let mut expected = vec![2];
        word(&mut expected, 7);
        word(&mut expected, 9);
        word(&mut expected, 11);
        let mut bytes = vec![0; TIP_LEN];
        let used = request.store(&mut bytes).unwrap();
        assert_eq!(used, 25);
        assert_eq!(&bytes[..used], expected.as_slice());
        assert!(
            matches!(TipIn::fetch(&expected), Some(TipIn::Guest { who, reply, ask })
            if who == TaskId::new(7) && reply == PieToken::mint(9) && ask == PieToken::mint(11))
        );
        assert!(
            TipIn::fetch(&expected[..9]).is_none(),
            "implicit legacy handoff must not rescan"
        );
        assert!(
            TipIn::fetch(&expected[..17]).is_none(),
            "reply-only handoff cannot bind a session"
        );
        expected.push(0);
        assert!(TipIn::fetch(&expected).is_none());
    }

    #[test]
    fn request_goldens_preserve_action_codes() {
        let road = Path::new("svc").to_path_buf();
        let root = vec![0; 9];
        let mut watch = vec![8, 3, b's', b'v', b'c'];
        word(&mut watch, 9);
        let mut list = vec![5];
        list.extend_from_slice(&root);
        let cases = [
            (
                Req::Find(EntryId::new(0)),
                Wire::Find(EntryId::new(0)),
                vec![3, 0, 0, 0, 0, 0, 0, 0, 0],
            ),
            (Req::List(Where::Root), Wire::List(Where::Root), list),
            (
                Req::Road(road.clone()),
                Wire::Road(road.clone()),
                vec![7, 3, b's', b'v', b'c'],
            ),
            (
                Req::Name(EntryId::new(0)),
                Wire::Name(EntryId::new(0)),
                vec![6, 0, 0, 0, 0, 0, 0, 0, 0],
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
            ),
        ];
        for (request, decoded, expected) in cases {
            let mut buffer = Req::EMPTY;
            let len = request.store(&mut buffer).unwrap();
            assert_eq!(&buffer[..len], expected);
            assert_eq!(Req::fetch(&expected), Some(decoded.clone()));
            let mut tailed = expected.clone();
            tailed.push(0);
            assert_eq!(Req::fetch(&tailed), None);
            for end in 0..expected.len() {
                assert_eq!(Req::fetch(&expected[..end]), None);
            }
        }
        let key = NamespaceKey::of("service");
        for (request, decoded) in [
            (
                Req::Resolve {
                    mark: key.mark(),
                    name: key.name().into(),
                },
                Wire::Resolve {
                    mark: key.mark(),
                    name: key.name().into(),
                },
            ),
            (
                Req::Relative {
                    base: Entry {
                        operator: TaskId::new(7),
                        target: EntryTarget::Root,
                    },
                    road: PathBuf::root(),
                },
                Wire::Relative {
                    base: Entry {
                        operator: TaskId::new(7),
                        target: EntryTarget::Root,
                    },
                    road: PathBuf::root(),
                },
            ),
        ] {
            let mut bytes = Req::EMPTY;
            let len = request.store(&mut bytes).unwrap();
            assert_eq!(Req::fetch(&bytes[..len]), Some(decoded));
            assert!(Req::fetch(&bytes[..len - 1]).is_none());
        }
    }

    #[test]
    fn reply_goldens_keep_entry_seed_list_and_name_shapes() {
        for (reply, expected) in [
            (Union::Status(DENIED), vec![8]),
            (Union::Entry(EntryId::new(0)), vec![0; 9]),
            (
                Union::Reference(Entry {
                    operator: TaskId::new(7),
                    target: EntryTarget::Root,
                }),
                vec![0, 7, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0],
            ),
            (
                Union::Reference(Entry {
                    operator: TaskId::new(7),
                    target: EntryTarget::Node(EntryId::new(0)),
                }),
                vec![0, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            ),
            (
                Union::Seed(PieToken::mint(9)),
                vec![0, 9, 0, 0, 0, 0, 0, 0, 0],
            ),
            (Union::Name("x".into()), vec![0, b'x']),
            (
                Union::WatchPath(PathBuf::try_new("a/b").unwrap()),
                vec![0, 3, b'a', b'/', b'b'],
            ),
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
                Union::WatchPath(road) => assert_eq!(received.watch_path(), Ok(road)),
                Union::List(ids) => assert_eq!(received.list(), Ok(ids)),
                Union::Reference(entry) => assert_eq!(received.reference(), Ok(entry)),
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
    fn resource_view_is_consumer_access_separate_from_transfer_policy() {
        assert_eq!(
            system_api::operator::ResourceView::new(env::Access::NONE),
            None
        );
        assert_eq!(
            system_api::operator::ResourceView::new(env::Access::FETCH)
                .map(system_api::operator::ResourceView::access),
            Some(env::Access::FETCH)
        );
        assert_eq!(
            system_api::operator::ResourceView::from_bits(
                env::Access::FETCH.bits().bits() | env::Permission::VEST.bits()
            ),
            None
        );
        let view = system_api::operator::ResourceView::new(env::Access::FETCH).unwrap();
        assert!(view.fits(env::Permission::FETCH | env::Permission::VEST));
        assert!(!view.fits(env::Permission::STORE | env::Permission::VEST));
    }

    #[test]
    fn injected_grant_descriptor_binds_exact_namespace_and_real_entry_issuer() {
        let descriptor = GrantDescriptor {
            id: 41,
            entry_operator: TaskId::new(8),
            entry_root: 0,
            entry_id: EntryId::new(12),
            entrance: PieToken::mint(99),
        };
        let mut bytes = [0; GrantDescriptor::LEN];
        let len = descriptor.store_at(&mut bytes, 0).unwrap();
        assert_eq!(len, GrantDescriptor::LEN);
        assert_eq!(GrantDescriptor::fetch_at(&bytes, 0).unwrap().0, descriptor);
        assert_eq!(descriptor_mark(0), Some(Mark::new(0x5359_534f_0000_0000)));
        assert_ne!(descriptor_mark(0), descriptor_mark(1));
        assert_eq!(descriptor_mark(31).is_some(), true);
        assert_eq!(descriptor_mark(32), None);
        assert!(GrantDescriptor::fetch_at(&bytes[..len - 1], 0).is_none());
    }

    #[test]
    fn static_namespace_declarations_are_canonical_and_bounded() {
        use crate::unit_operator::{Binding, Namespace, Scope};
        static BINDINGS: [Binding; 1] = [Binding {
            name: "ask",
            mark: env::Mark::new(44),
            view: env::Access::FETCH,
            permit: system_api::operator::Permit::Public,
        }];
        static NAMES: &[&str] = &["principal", "team/member"];
        let declaration = Namespace {
            key: NamespaceKey::of("test-namespace"),
            placement: crate::unit_operator::Placement::Shared,
            path: "svc/identity",
            scope: Scope::Names(NAMES),
            ops: system_api::operator::Ops::FIND,
            permit: Some(system_api::operator::Permit::Public),
            source_mark: None,
            bindings: &BINDINGS,
        };
        assert!(declaration.valid());
        assert!(
            !Namespace {
                path: "svc//identity",
                ..declaration
            }
            .valid()
        );
        assert!(
            !Namespace {
                scope: Scope::Names(&["../outside"]),
                ..declaration
            }
            .valid()
        );
        assert!(
            !Namespace {
                ops: system_api::operator::Ops::from_bits(1 << 12).unwrap(),
                ..declaration
            }
            .valid()
        );
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
        assert_eq!(fail_to_code(Some(Fail::Conflict)), 10);
        assert_eq!(code_to_fail(10), Some(Fail::Conflict));
        let key = NamespaceKey::of("service");
        assert_eq!(key.mark(), env::Mark::of("service"));
        assert_eq!(key.name(), "service");
    }

    #[test]
    fn sdk_separates_read_discovery_from_entry_capabilities() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../src/system/client/src/operator");
        let client = std::fs::read_to_string(root.join("face.rs")).unwrap();
        let module = std::fs::read_to_string(root.join("mod.rs")).unwrap();
        for method in [
            "find",
            "list",
            "seek",
            "resolve",
            "seek_from",
            "pane_from",
            "tile_from",
            "name",
            "watch",
        ] {
            assert!(
                client.contains(&format!("pub fn {method}(")),
                "Face::{method} missing"
            );
        }
        for method in [
            "part",
            "bind",
            "replace",
            "unpart",
            "read",
            "grant",
            "inspect_grant",
            "revoke",
        ] {
            assert!(
                module.contains(&format!("pub fn {method}("))
                    || std::fs::read_to_string(root.join("capability.rs"))
                        .unwrap()
                        .contains(&format!("pub fn {method}(")),
                "Capability::{method} missing"
            );
        }
        for method in ["part", "land", "trim"] {
            assert!(
                !client.contains(&format!("pub fn {method}(")),
                "legacy Face::{method} remains"
            );
        }
        let api = std::fs::read_to_string(root.join("../../../api/src/operator/mod.rs")).unwrap();
        assert!(!api.contains("pub enum Grant"));
        assert!(!api.contains("mod grant"));
        for source in [&client, &module] {
            assert!(!source.contains("Rein"));
            assert!(!source.contains("granted_berth"));
            assert!(!source.contains("fn rein("));
        }
    }
}
