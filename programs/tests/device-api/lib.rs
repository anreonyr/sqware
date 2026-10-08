#![allow(dead_code)]

#[cfg(test)]
mod tests {
    use env::wire::Span as _;
    use env::{Mark, PieToken, TaskId};
    use terminal_api::frame as terminal;
    use wire::message::Message;

    fn token(value: u64) -> PieToken {
        PieToken::from_bytes(&value.to_le_bytes()).unwrap()
    }

    #[test]
    fn terminal_command_and_reply_keep_their_fixed_layouts() {
        let command = terminal::Command {
            op: terminal::ATTACH,
            task: TaskId::new(0x0807_0605_0403_0201),
            authority: token(0x1817_1615_1413_1211),
            back: token(0x2827_2625_2423_2221),
        };
        assert_eq!(terminal::Command::LEN, 25);
        let mut bytes = [0; terminal::Command::LEN];
        let n = command.store_at(&mut bytes, 0).unwrap();
        assert_eq!(n, bytes.len());
        assert_eq!(
            &bytes,
            &[
                1, 1, 2, 3, 4, 5, 6, 7, 8, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x21,
                0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28,
            ]
        );
        let (decoded, end) = terminal::Command::fetch_at(&bytes, 0).unwrap();
        assert_eq!(end, n);
        assert_eq!(decoded.op, command.op);
        assert_eq!(decoded.task, command.task);
        assert_eq!(decoded.authority, command.authority);
        assert_eq!(decoded.back, command.back);

        let reply = terminal::Reply {
            status: 0,
            authority: token(0x0807_0605_0403_0201),
        };
        assert_eq!(terminal::Reply::LEN, 9);
        let mut bytes = [0; terminal::Reply::LEN];
        let n = reply.store_at(&mut bytes, 0).unwrap();
        assert_eq!(&bytes[..n], &[0, 1, 2, 3, 4, 5, 6, 7, 8]);
        let (decoded, end) = terminal::Reply::fetch_at(&bytes[..n], 0).unwrap();
        assert_eq!(end, n);
        assert_eq!(decoded.status, reply.status);
        assert_eq!(decoded.authority, reply.authority);
    }

    #[test]
    fn terminal_input_encodes_only_its_payload_and_rejects_short_or_long_frames() {
        let input = terminal::Input::data(&[0xaa, 0xbb, 0xcc]).unwrap();
        let mut bytes = terminal::Input::EMPTY;
        let n = input.store(&mut bytes).unwrap();
        assert_eq!(&bytes[..n], &[terminal::DATA, 3, 0, 0, 0, 0xaa, 0xbb, 0xcc]);
        let decoded = terminal::Input::fetch(&bytes[..n]).unwrap();
        assert_eq!(decoded.kind, terminal::DATA);
        assert_eq!(decoded.bytes(), input.bytes());
        assert!(terminal::Input::fetch(&bytes[..n - 1]).is_none());
        assert!(terminal::Input::fetch(&bytes[..n + 1]).is_none());

        let eof = terminal::Input::eof();
        let n = eof.store(&mut bytes).unwrap();
        assert_eq!(&bytes[..n], &[terminal::EOF, 0, 0, 0, 0]);
        let decoded = terminal::Input::fetch(&bytes[..n]).unwrap();
        assert_eq!(decoded.kind, eof.kind);
        assert!(decoded.bytes().is_empty());
        assert!(terminal::Input::data(&[]).is_none());
        assert!(terminal::Input::data(&[0; terminal::MAX + 1]).is_none());
    }

    #[test]
    fn router_occupy_keeps_its_five_byte_shape_and_exact_decode_boundary() {
        let request = router_api::frame::Occupy::of(0x1122_3344);
        let mut bytes = router_api::frame::Occupy::EMPTY;
        let n = request.store(&mut bytes).unwrap();
        assert_eq!(router_api::frame::Occupy::LEN, 5);
        assert_eq!(
            &bytes[..n],
            &[router_api::frame::OCCUPY, 0x44, 0x33, 0x22, 0x11]
        );
        assert_eq!(
            router_api::frame::Occupy::fetch(&bytes[..n]),
            Some(0x1122_3344)
        );
        assert!(router_api::frame::Occupy::fetch(&bytes[..n - 1]).is_none());
        let mut extra = [0; router_api::frame::Occupy::LEN + 1];
        extra[..n].copy_from_slice(&bytes[..n]);
        assert!(router_api::frame::Occupy::fetch(&extra).is_none());
        bytes[0] = 0xff;
        assert!(router_api::frame::Occupy::fetch(&bytes[..n]).is_none());
    }

    #[test]
    fn router_handoff_carries_exact_recipient_seeds_in_a_distinct_frame() {
        let request = router_api::frame::OccupyLane::of(
            0x1122_3344,
            token(0x0807_0605_0403_0201),
            token(0x1817_1615_1413_1211),
        );
        assert_eq!(router_api::frame::Occupy::LEN, 5);
        assert_eq!(router_api::frame::OccupyLane::LEN, 21);
        let mut bytes = router_api::frame::OccupyLane::EMPTY;
        let n = request.store(&mut bytes).unwrap();
        assert_eq!(
            &bytes[..n],
            &[
                router_api::frame::OCCUPY_LANE,
                0x44,
                0x33,
                0x22,
                0x11,
                1,
                2,
                3,
                4,
                5,
                6,
                7,
                8,
                0x11,
                0x12,
                0x13,
                0x14,
                0x15,
                0x16,
                0x17,
                0x18,
            ]
        );
        assert_eq!(
            router_api::frame::OccupyLane::fetch(&bytes[..n]),
            Some((
                0x1122_3344,
                token(0x0807_0605_0403_0201),
                token(0x1817_1615_1413_1211),
            ))
        );
        assert!(router_api::frame::OccupyLane::fetch(&bytes[..n - 1]).is_none());
        assert!(router_api::frame::OccupyLane::fetch(&[0; 5]).is_none());
        let mut extra = [0; router_api::frame::OccupyLane::LEN + 1];
        extra[..n].copy_from_slice(&bytes[..n]);
        assert!(router_api::frame::OccupyLane::fetch(&extra).is_none());
        bytes[0] = router_api::frame::OCCUPY;
        assert!(router_api::frame::OccupyLane::fetch(&bytes[..n]).is_none());

        let reply =
            router_api::frame::OccupyReply::of(router_api::frame::OK, token(0x2827_2625_2423_2221));
        assert_eq!(router_api::frame::OccupyReply::LEN, 9);
        let mut bytes = router_api::frame::OccupyReply::EMPTY;
        let n = reply.store(&mut bytes).unwrap();
        assert_eq!(
            &bytes[..n],
            &[
                router_api::frame::OK,
                0x21,
                0x22,
                0x23,
                0x24,
                0x25,
                0x26,
                0x27,
                0x28
            ]
        );
        assert_eq!(
            router_api::frame::OccupyReply::fetch(&bytes[..n]),
            Some((router_api::frame::OK, token(0x2827_2625_2423_2221)))
        );
        assert!(router_api::frame::OccupyReply::fetch(&bytes[..n - 1]).is_none());
        assert!(router_api::frame::OccupyReply::fetch(&[router_api::frame::DENIED; 1]).is_none());
        let failure = router_api::frame::OccupyReply::of(router_api::frame::DENIED, PieToken::NONE);
        let n = failure.store(&mut bytes).unwrap();
        assert_eq!(
            &bytes[..n],
            &[router_api::frame::DENIED, 0, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn device_marks_are_stable_and_all_provider_registries_are_collision_free() {
        assert_eq!(terminal_api::marks::ENTRY, Mark::of("terminal-attach"));
        assert_eq!(
            terminal_api::marks::AUTHORITY,
            Mark::of("terminal-authority")
        );
        assert_eq!(terminal_api::marks::BACK, Mark::of("terminal-back"));
        assert_eq!(terminal_api::marks::INPUT, Mark::of("terminal-input"));
        assert_eq!(terminal_api::marks::OUTPUT, Mark::of("terminal-output"));
        assert_eq!(terminal_api::marks::CONTROL, Mark::of("terminal-control"));
        assert_eq!(router_api::ENTRY_MARK, Mark::of("entry"));
        assert_eq!(router_api::LINE_BACK, Mark::of("line-back"));
        assert_eq!(router_api::LINE_MARK, Mark::of("line"));
        assert_eq!(router_api::frame::BACK_MARK, router_api::LINE_BACK);
        assert_eq!(router_api::frame::LANE, router_api::LANE);

        assert_eq!(hub_api::PUBLICATIONS, ["bond", "list", "claim"]);
        assert_eq!(terminal_api::PUBLICATIONS, ["attach"]);
        assert_eq!(router_api::PUBLICATIONS, ["router"]);

        let registries: &[&[&[env::marks::Definition]]] = &[
            &[system_api::loader::REGISTRY],
            system_api::identity::REGISTRY,
            system_api::operator::REGISTRY,
            system_api::control::REGISTRY,
            hub_api::REGISTRY,
            terminal_api::REGISTRY,
            router_api::REGISTRY,
        ];
        assert_eq!(env::marks::conflict_between(registries), None);
    }
}
