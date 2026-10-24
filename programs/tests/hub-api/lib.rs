#![allow(dead_code)]
extern crate alloc;

#[cfg(test)]
mod tests {
    use alloc::{string::String};
    use env::{Mark, PieToken};
    use hub_api::{frame, Grant};
    use wire::message::Message;

    fn token(raw: u64) -> PieToken {
        PieToken::from_bytes(&raw.to_le_bytes()).unwrap()
    }

    #[test]
    fn binding_marks_preserve_full_width_serials_without_address_limits() {
        assert_eq!(hub_api::binding_mark(1), Mark::new(0x5043_4900_0000_0001));
        assert_eq!(hub_api::window_mark(1), Mark::new(0x5749_4e00_0000_0001));
        assert_eq!(hub_api::coord_mark(1), Mark::new(0x434f_4f00_0000_0001));
        assert_eq!(hub_api::binding_mark(u64::MAX), Mark::new(0x5043_48ff_ffff_ffff));
        assert_eq!(hub_api::window_mark(u64::MAX), Mark::new(0x5749_4dff_ffff_ffff));
        assert_eq!(hub_api::coord_mark(u64::MAX), Mark::new(0x434f_4eff_ffff_ffff));
        for mark in [hub_api::binding_mark, hub_api::window_mark, hub_api::coord_mark] {
            let serials = [1, 64, 511, 512, 126975, 126976, u64::MAX];
            for (at, serial) in serials.iter().enumerate() {
                for other in &serials[at + 1..] { assert_ne!(mark(*serial), mark(*other)); }
            }
        }
    }

    #[test]
    fn marks_grants_and_actions_keep_their_fixed_values() {
        assert_eq!([frame::BOND, frame::LIST, frame::CLAIM], [1, 2, 3]);
        assert_eq!(Grant::ALL.map(Grant::at), [1, 2, 3, 4, 5]);
        assert_eq!(Grant::ALL.map(Grant::name), ["bond", "list", "claim", "dma", "retire"]);
        assert_eq!(Grant::ALL.map(Grant::mark), [
            Mark::of("hub-entry-bond"), Mark::of("hub-entry-list"), Mark::of("hub-entry-claim"),
            Mark::of("hub-entry-dma"), Mark::of("hub-entry-retire"),
        ]);
        assert_eq!(hub_api::marks::DECLARATIONS.len(), 2);
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
    fn claim_preserves_layout() {
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

    }
}
