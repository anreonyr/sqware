use env::PieToken;
use hub_api::{DmaWire, Enroll, DMA_ENROLL};
use wire::Message;

#[test]
fn enrollment_preserves_the_entire_opaque_key_without_adding_a_source_field() {
    let mut encoded = Vec::new();
    for key in [0, u64::from(u32::MAX), 1 << 32, u64::MAX] {
        let request = DmaWire::Enroll(Enroll {
            op: DMA_ENROLL, key, line: 7, name: "disk".into(), class: "block".into(),
            back: PieToken::mint(31),
        });
        let mut bytes = <DmaWire as Message>::EMPTY;
        let len = request.store(&mut bytes).unwrap();
        let Some(DmaWire::Enroll(decoded)) = DmaWire::fetch(&bytes[..len]) else {
            panic!("valid registration frame");
        };
        assert_eq!(decoded.key, key);
        assert_eq!(decoded.line, 7);
        assert_eq!(decoded.name, "disk");
        assert_eq!(decoded.class, "block");
        assert_eq!(decoded.back, PieToken::mint(31));
        for end in 0..len {
            assert!(DmaWire::fetch(&bytes[..end]).is_none());
        }
        let mut trailing = bytes[..len].to_vec();
        trailing.push(0);
        assert!(DmaWire::fetch(&trailing).is_none());
        encoded.push(bytes[..len].to_vec());
    }
    assert_ne!(encoded[0], encoded[2], "a zero low word must not erase high key bits");
}
