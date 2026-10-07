#![allow(dead_code)]
extern crate alloc;

#[cfg(test)]
mod tests {
    use alloc::{string::String, vec};
    use env::{Mark, PieToken, TaskId};
    use hub_api::{activation::Activate, frame, Grant};
    use wire::message::Message;

    fn token(raw: u64) -> PieToken {
        PieToken::from_bytes(&raw.to_le_bytes()).unwrap()
    }

    #[test]
    fn marks_grants_and_actions_keep_their_fixed_values() {
        assert_eq!([frame::BOND, frame::LIST, frame::CLAIM], [1, 2, 3]);
        assert_eq!(Grant::ALL.map(Grant::at), [1, 2, 3]);
        assert_eq!(Grant::ALL.map(Grant::name), ["bond", "list", "claim"]);
        assert_eq!(Grant::ALL.map(Grant::mark), [
            Mark::of("hub-entry-bond"), Mark::of("hub-entry-list"), Mark::of("hub-entry-claim"),
        ]);
        assert_eq!(hub_api::marks::DECLARATIONS.len(), 4);
        assert_eq!(env::marks::conflict(hub_api::REGISTRY), None);
    }

    #[test]
    fn variable_request_frames_keep_exact_bytes_and_reject_trailing_data() {
        let back = token(0x0102_0304_0506_0708);
        let bond = frame::Bond::of(String::from("gpu"), back);
        let mut bytes = frame::Bond::EMPTY;
        let len = bond.store(&mut bytes).unwrap();
        assert_eq!(&bytes[..len], &[1, 3, b'g', b'p', b'u', 8, 7, 6, 5, 4, 3, 2, 1]);
        assert_eq!(frame::Bond::fetch(&bytes[..len]), Some(bond));
        assert!(frame::Bond::fetch(&bytes[..len + 1]).is_none());

        let list = frame::ListReq::of(String::from("net"), 0x1122_3344, back);
        let mut bytes = frame::ListReq::EMPTY;
        let len = list.store(&mut bytes).unwrap();
        assert_eq!(&bytes[..len], &[
            2, 3, b'n', b'e', b't', 0x44, 0x33, 0x22, 0x11, 8, 7, 6, 5, 4, 3, 2, 1,
        ]);
        assert_eq!(frame::ListReq::fetch(&bytes[..len]), Some(list));
    }

    #[test]
    fn claim_and_variable_activation_preserve_layout_and_count_limit() {
        let back = token(0x0102_0304_0506_0708);
        let sensor = token(0x1112_1314_1516_1718);
        let claim = frame::Claim::of(2, 0x1122_3344, 0x5566_7788, sensor, back);
        let mut bytes = frame::Claim::EMPTY;
        let len = claim.store(&mut bytes).unwrap();
        assert_eq!(&bytes[..len], &[
            3, 2, 0x44, 0x33, 0x22, 0x11, 0x88, 0x77, 0x66, 0x55,
            0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11,
            8, 7, 6, 5, 4, 3, 2, 1,
        ]);
        assert_eq!(frame::Claim::fetch(&bytes[..len]), Some(claim));

        let coalition = system_api::identity::CoalitionId::new(TaskId::new(7), 9);
        let coalitions = vec![coalition; hub_api::activation::ACTIVATE_MAX];
        let activation = Activate::of(TaskId::new(11), &coalitions, back).unwrap();
        let mut bytes = Activate::EMPTY;
        let len = activation.store(&mut bytes).unwrap();
        let decoded = Activate::fetch(&bytes[..len]).unwrap();
        assert_eq!(decoded.task, activation.task);
        assert_eq!(decoded.back, activation.back);
        assert_eq!(decoded.len(), activation.len());
        assert_eq!(decoded.coalition(0), Some(coalition));
        assert_eq!(activation.len(), hub_api::activation::ACTIVATE_MAX);
        assert!(Activate::of(TaskId::new(11), &coalitions[..coalitions.len() - 1], back).is_some());
        let too_many = vec![coalition; hub_api::activation::ACTIVATE_MAX + 1];
        assert!(Activate::of(TaskId::new(11), &too_many, back).is_none());
    }
}
