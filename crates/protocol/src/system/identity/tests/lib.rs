#![allow(dead_code)]
#[path = "../frame/mod.rs"]
pub mod frame;
#[path = "../grant.rs"]
mod grant;
#[path = "../limits.rs"]
mod limits;
#[path = "../marks.rs"]
pub mod marks;

#[cfg(test)]
mod tests {
    use super::{frame::*, frame::vocab::*, grant::*};
    use wire::message::Message;
    use env::{PieToken, TaskId};

    fn p(slot: u64) -> PrincipalId {
        PrincipalId::new(TaskId::new(5), slot)
    }
    fn c(slot: u64) -> CoalitionId {
        CoalitionId::new(TaskId::new(5), slot)
    }
    fn subject() -> Subject {
        Subject::new(p(0), &[c(0), c(9)]).unwrap()
    }
    fn roundtrip(wire: Wire) {
        let mut bytes = [0; MAX_FRAME];
        let back = PieToken::from_bytes(&17u64.to_le_bytes()).unwrap();
        let n = wire.store(back, &mut bytes).unwrap();
        let mut envelope = [0; MAX_FRAME];
        let request = Request(wire, back);
        assert_eq!(request.store(&mut envelope), Some(n));
        assert_eq!(&envelope[..n], &bytes[..n]);
        assert_eq!(Request::fetch(&envelope[..n]), Wire::take(&bytes[..n]));
        assert_eq!(&bytes[..8], &17u64.to_le_bytes());
        assert_eq!(bytes[8], Grant::of_wire(&wire));
        assert_eq!(Wire::take(&bytes[..n]), Some((Some(wire), back)));
        assert_eq!(Wire::take(&bytes[..n + 1]), Some((None, back)));
        for len in 0..n {
            assert!(!matches!(Wire::take(&bytes[..len]), Some((Some(_), _))));
        }
    }
    #[test]
    fn all_actions_and_install_shapes_roundtrip_strictly() {
        let task = TaskId::new(8);
        let s = subject();
        let k = Some(Cursor {
            target: PageTarget::Members(c(0)),
            revision: 3,
            after: 0,
        });
        let actions = [
            Wire::Resolve(task),
            Wire::Matches(task, Selector::Exact(p(0))),
            Wire::Same(task, task),
            Wire::Sire(p(0)),
            Wire::Heir(p(0), p(1)),
            Wire::Amid(p(0), c(0)),
            Wire::Members(c(0), k),
            Wire::Memberships(p(0), None),
            Wire::Adopt(s),
            Wire::Waive,
            Wire::Restrict(s),
            Wire::Derive(p(0)),
            Wire::Found,
            Wire::Admit(c(0), p(0)),
            Wire::Expel(c(0), p(0)),
            Wire::Bind(task, Install::Authorized(s)),
            Wire::Unbind(task),
        ];
        for (i, wire) in actions.into_iter().enumerate() {
            assert_eq!(Grant::for_wire(&wire), Grant::ALL[i]);
            assert_eq!(Grant::of_wire(&wire), (i + 1) as u8);
            assert_eq!(Grant::ALL[i].at(), (i + 1) as u8);
            assert_eq!(Grant::ALL[i].index(), i);
            assert_eq!(Grant::from_action((i + 1) as u8), Some(Grant::ALL[i]));
            assert_eq!(grant_of(Grant::ALL[i].mark()), Some(Grant::ALL[i]));
            roundtrip(wire);
        }
        roundtrip(Wire::Bind(task, Install::Inherit { parent: task }));
        roundtrip(Wire::Bind(
            task,
            Install::Restrict {
                parent: task,
                subject: s,
            },
        ));
        roundtrip(Wire::Matches(task, Selector::DescendantOf(p(0))));
        roundtrip(Wire::Matches(task, Selector::MemberOf(c(0))));
    }
    #[test]
    fn replies_are_strict_and_zero_slots_are_real() {
        let s = subject();
        let k = Some(Cursor {
            target: PageTarget::Members(c(0)),
            revision: 0,
            after: 0,
        });
        let values = [
            Reply::Unit,
            Reply::Binding(None),
            Reply::Binding(Some(Binding {
                origin: s,
                current: s,
            })),
            Reply::Principal(None),
            Reply::Principal(Some(p(0))),
            Reply::Coalition(c(0)),
            Reply::Match(Match::Yes),
            Reply::Match(Match::No),
            Reply::Match(Match::Unbound),
            Reply::Bool(true),
            Reply::Bool(false),
            Reply::Members(Page::new(&[p(0)], k).unwrap()),
            Reply::Memberships(Page::new(&[c(0)], None).unwrap()),
            Reply::Fail(Fail::Changed),
        ];
        for value in values {
            let mut bytes = Reply::EMPTY;
            let n = value.store(&mut bytes).unwrap();
            assert_eq!(Reply::fetch(&bytes[..n]), Some(value));
            assert_eq!(Reply::fetch(&bytes[..n + 1]), None);
            for len in 0..n {
                assert_eq!(Reply::fetch(&bytes[..len]), None);
            }
        }
        assert_eq!(Reply::fetch(&[0, 5, 2]), None);
        assert_eq!(Reply::fetch(&[0, 1, 2]), None);
        assert_eq!(Reply::fetch(&[255]), None);
        for code in 1..=10 {
            let fail = code_to_fail(code).unwrap();
            assert_eq!(fail_to_code(Some(fail)), code);
        }
    }
    #[test]
    fn generated_failure_codes_keep_the_single_byte_reply() {
        let failures = [
            Fail::Bad, Fail::UnknownPrincipal, Fail::UnknownCoalition, Fail::WrongAuthority,
            Fail::Denied, Fail::NotManager, Fail::NotNarrower, Fail::NotEligible,
            Fail::Full, Fail::Changed,
        ];
        assert_eq!(fail_to_code(None), 0);
        assert_eq!(code_to_fail(0), None);
        for (index, fail) in failures.into_iter().enumerate() {
            let code = (index + 1) as u8;
            assert_eq!(fail_to_code(Some(fail)), code);
            assert_eq!(code_to_fail(code), Some(fail));
            let mut out = Reply::EMPTY;
            let len = Reply::Fail(fail).store(&mut out).unwrap();
            assert_eq!(&out[..len], &[code]);
            assert_eq!(Reply::fetch(&[code]), Some(Reply::Fail(fail)));
            assert_eq!(Reply::fetch(&[code, 0]), None);
        }
        for code in 11..=255 { assert_eq!(code_to_fail(code), None); }
    }
    #[test]
    fn frame_layouts_match_explicit_wire_goldens() {
        let back = PieToken::from_bytes(&17u64.to_le_bytes()).unwrap();
        let subject = Subject::new(p(0), &[c(9)]).unwrap();
        let request = Wire::Bind(TaskId::new(8), Install::Restrict { parent: TaskId::new(7), subject });
        let mut bytes = [0; MAX_FRAME];
        let len = request.store(back, &mut bytes).unwrap();
        let mut expected = Vec::new();
        expected.extend_from_slice(&17u64.to_le_bytes());
        expected.push(16);
        expected.extend_from_slice(&8u64.to_le_bytes());
        expected.push(2);
        for word in [7u64, 5, 0] { expected.extend_from_slice(&word.to_le_bytes()); }
        expected.push(1);
        for word in [5u64, 9] { expected.extend_from_slice(&word.to_le_bytes()); }
        assert_eq!(&bytes[..len], expected.as_slice());
        assert_eq!(Wire::take(&expected), Some((Some(request), back)));

        let binding = Binding {
            origin: Subject::new(p(0), &[c(0), c(9)]).unwrap(),
            current: Subject::new(p(1), &[c(9)]).unwrap(),
        };
        let reply = Reply::Binding(Some(binding));
        let len = reply.store(&mut bytes).unwrap();
        let mut expected = vec![0, 1, 1];
        for word in [5u64, 0] { expected.extend_from_slice(&word.to_le_bytes()); }
        expected.push(2);
        for word in [5u64, 0, 5, 9, 5, 1] { expected.extend_from_slice(&word.to_le_bytes()); }
        expected.push(1);
        for word in [5u64, 9] { expected.extend_from_slice(&word.to_le_bytes()); }
        assert_eq!(&bytes[..len], expected.as_slice());
        assert_eq!(Reply::fetch(&expected), Some(reply));

        let page = Page::new(&[p(0), p(9)], Some(Cursor {
            target: PageTarget::Members(c(0)), revision: 3, after: 9,
        })).unwrap();
        let reply = Reply::Members(page);
        let len = reply.store(&mut bytes).unwrap();
        let mut expected = vec![0, 6, 2];
        for word in [5u64, 0, 5, 9] { expected.extend_from_slice(&word.to_le_bytes()); }
        expected.extend_from_slice(&[1, 0]);
        for word in [5u64, 0, 3, 9] { expected.extend_from_slice(&word.to_le_bytes()); }
        assert_eq!(&bytes[..len], expected.as_slice());
        assert_eq!(Reply::fetch(&expected), Some(reply));
    }
    #[test]
    fn sets_pages_and_sources_never_truncate() {
        assert_eq!(CoalitionSet::new(&[c(0), c(0)]), Err(Fail::Bad));
        assert_eq!(CoalitionSet::new(&[c(0); 17]), Err(Fail::Full));
        assert_eq!(
            CoalitionSet::new(&[c(0), CoalitionId::new(TaskId::new(6), 1)]),
            Err(Fail::WrongAuthority)
        );
        let ids = core::array::from_fn::<_, 16, _>(|i| c(i as u64));
        let mut set = CoalitionSet::new(&ids).unwrap();
        assert_eq!(set.len(), 16);
        set.remove(c(0));
        assert!(!set.contains(c(0)));
        assert_eq!(set.len(), 15);
        assert_eq!(
            Subject::new(PrincipalId::new(TaskId::new(6), 0), &[c(0)]),
            Err(Fail::WrongAuthority)
        );
        assert_eq!(Page::new(&[p(0); 17], None), Err(Fail::Full));
        assert_eq!(Page::new(&[p(1), p(0)], None), Err(Fail::Bad));
        assert_eq!(
            Page::<PrincipalId>::new(
                &[],
                Some(Cursor {
                    target: PageTarget::Members(c(0)),
                    revision: 0,
                    after: 0
                })
            ),
            Err(Fail::Bad)
        );
        assert!(
            Wire::Members(
                c(0),
                Some(Cursor {
                    target: PageTarget::Members(c(1)),
                    revision: 0,
                    after: 0
                })
            )
            .store(PieToken::mint(1), &mut [0; MAX_FRAME])
            .is_none()
        );
        assert!(
            Wire::Heir(p(0), PrincipalId::new(TaskId::new(6), 0))
                .store(PieToken::mint(1), &mut [0; MAX_FRAME])
                .is_none()
        );
    }
    #[test]
    fn malformed_subject_order_count_and_source_are_rejected() {
        let mut bytes = [0; MAX_FRAME];
        let n = Wire::Adopt(subject())
            .store(PieToken::mint(1), &mut bytes)
            .unwrap();
        // Header=9, principal=16, count=1, full coalition IDs=16 each.
        let original = bytes;
        bytes[25] = 17;
        assert!(matches!(Wire::take(&bytes[..n]), Some((None, _))));
        bytes = original;
        bytes[42..58].copy_from_slice(&original[26..42]);
        assert!(matches!(Wire::take(&bytes[..n]), Some((None, _))));
        bytes = original;
        bytes[26..34].copy_from_slice(&6u64.to_le_bytes());
        assert!(matches!(Wire::take(&bytes[..n]), Some((None, _))));
        bytes = original;
        bytes[8] = 255;
        assert!(matches!(Wire::take(&bytes[..n]), Some((None, _))));
    }
    #[test]
    fn metadata_and_selector_field_are_consistent() {
        use env::wire::Field;
        for (i, g) in Grant::ALL.into_iter().enumerate() {
            assert_eq!(g.index(), i);
            assert_eq!(g.at(), (i + 1) as u8);
            assert_eq!(g.mark(), env::Mark::of(&format!("identity-{}", g.name())));
            assert_eq!(Grant::from_action(g.at()), Some(g));
            // 面名要**恰好一段**：门牌那条路由 `DIR.try_join(g.name())` 拼出，名字里带 `/` 或多出一段都不认。
            assert!(!g.name().is_empty() && !g.name().contains('/'));
            assert_ne!(g.mark(), BACK);
            for other in Grant::ALL {
                if g != other {
                    assert_ne!(g.mark(), other.mark());
                }
            }
        }
        assert_eq!(Grant::from_action(0), None);
        assert_eq!(Grant::from_action(18), None);
        assert_eq!(grant_of(BACK), None);
        let s = Selector::MemberOf(c(0));
        let mut bytes = [0; 17];
        s.store(&mut bytes);
        assert_eq!(Selector::fetch(&bytes), Some(s));
        assert_eq!(Selector::fetch(&bytes[..16]), None);
        bytes[0] = 3;
        assert_eq!(Selector::fetch(&bytes), None);
    }
}
