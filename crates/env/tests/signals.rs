use env::{Bit, Bits, MailCondition, Wire};
#[test]
fn signal_conditions_roundtrip_every_machine_bit() {
    for index in 0..usize::BITS as usize {
        let bit = Bit::of(index).unwrap();
        let condition = MailCondition::Signal(bit);
        assert_eq!(MailCondition::of(condition.wire()), Some(condition));
        let mut registers = [0; 6];
        let mut offset = 0;
        condition.pack(&mut registers, &mut offset);
        let mut offset = 0;
        assert_eq!(
            MailCondition::unpack(&registers, &mut offset).unwrap(),
            condition
        );
        assert_eq!(bit.bits().get(), 1usize << index);
    }
    assert!(Bit::of(usize::BITS as usize).is_none());
    assert!(MailCondition::of(3 + usize::BITS as usize).is_none());
    assert!(MailCondition::of(usize::MAX).is_none());
}
#[test]
fn zero_masks_are_rejected_and_bits_iterate_once() {
    assert!(Bits::of(0).is_none());
    let bits = Bits::of(0b10101).unwrap();
    assert_eq!(
        bits.iter().map(|b| b.index()).collect::<Vec<_>>(),
        vec![0, 2, 4]
    );
    let mut registers = [0; 6];
    let mut offset = 0;
    assert!(Bits::unpack(&registers, &mut offset).is_err());
    let mut offset = 0;
    bits.pack(&mut registers, &mut offset);
    let mut offset = 0;
    assert_eq!(Bits::unpack(&registers, &mut offset).unwrap(), bits);
}
