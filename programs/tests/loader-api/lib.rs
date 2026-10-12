#![allow(dead_code)]
extern crate alloc;
pub use system_api::loader as frame;

#[cfg(test)]
mod generation;

#[cfg(test)]
mod tests {
    use env::{PieToken, TaskId, wire::Span};
    use system_api::{control::construction, loader};
    use wire::Message;

    fn token(raw: u64) -> PieToken {
        PieToken::from_bytes(&raw.to_le_bytes()).unwrap()
    }

    fn claim() -> loader::Claim {
        loader::Claim {
            op: loader::CLAIM,
            task: TaskId::new(0x0102030405060708),
            back: token(0x1112131415161718),
        }
    }

    #[test]
    fn loader_exposes_only_claim_and_keeps_its_stable_wire_identity() {
        assert_eq!(loader::DIR, "loader");
        assert_eq!(loader::CLAIM, 1);
        assert_eq!(loader::Claim::LEN, 17);
        assert_eq!(loader::Grant::Claim.at(), 1);
        assert_eq!(loader::Grant::from_action(1), Some(loader::Grant::Claim));
        assert_eq!(loader::Grant::from_action(0), None);
        assert_eq!(loader::Grant::from_action(2), None);
        assert_eq!(
            loader::grant_of(loader::Grant::Claim.mark()),
            Some(loader::Grant::Claim)
        );
    }

    #[test]
    fn claim_round_trips_with_its_operation_grant_and_reply_route() {
        let request = loader::Wire::Claim(claim());
        let mut bytes = <loader::Wire as Message>::EMPTY;
        let len = request.store(&mut bytes).unwrap();
        assert_eq!(len, loader::Claim::LEN);
        assert_eq!(bytes[0], loader::CLAIM);
        assert_eq!(loader::Grant::for_wire(&request), loader::Grant::Claim);
        let decoded = <loader::Wire as Message>::fetch(&bytes[..len]).unwrap();
        let loader::Wire::Claim(decoded) = decoded;
        assert_eq!(decoded.task, claim().task);
        assert_eq!(decoded.back, claim().back);
        assert!(<loader::Wire as Message>::fetch(&bytes[..len - 1]).is_none());
    }

    #[test]
    fn claim_rejects_other_operation_codes_and_trailing_data() {
        let mut bytes = <loader::Wire as Message>::EMPTY;
        let len = loader::Wire::Claim(claim()).store(&mut bytes).unwrap();
        for op in [0, 2, 127, 255] {
            let mut malformed = bytes;
            malformed[0] = op;
            assert!(<loader::Wire as Message>::fetch(&malformed[..len]).is_none());
        }
        let mut trailing = bytes[..len].to_vec();
        trailing.push(0);
        assert!(<loader::Wire as Message>::fetch(&trailing).is_none());
    }

    #[test]
    fn replies_round_trip_claim_state_and_startup_metadata() {
        let said = loader::Said {
            status: wire::OK,
            state: construction::COMPLETED,
            team: 0x2122232425262728,
            task: TaskId::new(0x3132333435363738),
            startup: loader::StartupMetadata {
                resolve: token(0x4142434445464748),
            },
        };
        let mut bytes = [0; loader::Said::LEN];
        let len = said.store(&mut bytes).unwrap();
        assert_eq!(len, loader::Said::LEN);
        let decoded = loader::Said::fetch(&bytes[..len]).unwrap();
        assert_eq!(decoded.status, wire::OK);
        assert_eq!(decoded.state, construction::COMPLETED);
        assert_eq!((decoded.team, decoded.task), (said.team, said.task));
        assert_eq!(decoded.startup.resolve, said.startup.resolve);
    }

    #[test]
    fn construction_image_is_a_control_input_not_a_loader_build_request() {
        let mut args = [0; loader::MAX_ARGS];
        args[0] = 0x5152535455565758;
        args[1] = 0x6162636465666768;
        let image = construction::Image {
            image: token(0x0102030405060708),
            offset: 0x1112131415161718,
            len: 0x2122232425262728,
            stack: 0x3132333435363738,
            count: 2,
            args,
        };
        let input = construction::Input::Image(image);
        let mut bytes = [0; 802];
        let len = input.store_at(&mut bytes, 0).unwrap();
        assert_eq!(bytes[0], 2);
        let (decoded, end) = construction::Input::fetch_at(&bytes[..len], 0).unwrap();
        assert_eq!(end, len);
        let construction::Input::Image(decoded) = decoded else {
            panic!("image input");
        };
        assert_eq!(decoded.image, image.image);
        assert_eq!(
            (decoded.offset, decoded.len, decoded.stack),
            (image.offset, image.len, image.stack)
        );
        assert_eq!(decoded.count, 2);
        assert_eq!(&decoded.args[..2], &image.args[..2]);
        assert!(decoded.args[2..].iter().all(|arg| *arg == 0));
    }
}
