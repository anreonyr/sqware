#[mold::interface(id = "test.interface.v1")]
mod sample {
    #[channels]
    pub enum Channel {
        #[channel(key = "response")]
        Renamed,
    }
    #[grants]
    pub enum Grant {
        #[grant(code = 91, key = "second")]
        Second,
        #[grant(code = 7, key = "first")]
        First,
    }
    #[requests]
    pub enum Wire {
        #[operation(code = 42, grant = First, frame = Later)]
        Later { value: u64 },
        #[operation(code = 3, grant = First, frame = Earlier)]
        Earlier { value: u64 },
        #[operation(code = 5, grant = Second, frame = Other)]
        Other { value: u64 },
    }
    #[reply]
    pub struct Reply { pub value: u64 }
}

#[test]
fn explicit_codes_and_stable_keys_are_independent_of_declaration_order() {
    use sample::*;
    assert_eq!((LATER, EARLIER, OTHER), (42, 3, 5));
    assert_eq!((Grant::Second.at(), Grant::Second.index()), (91, 0));
    assert_eq!((Grant::First.at(), Grant::First.index()), (7, 1));
    assert_eq!(Grant::from_action(91), Some(Grant::Second));
    assert_eq!(Grant::from_action(7), Some(Grant::First));
    assert_eq!(Grant::from_action(1), None);
    let value = Wire::Later(Later { op: 42, value: 0x0102030405060708 });
    let mut bytes = [0; 9];
    assert_eq!(value.store(&mut bytes), Some(9));
    assert_eq!(bytes, [42, 8, 7, 6, 5, 4, 3, 2, 1]);
    assert_eq!(Grant::for_wire(&value), Grant::First);
    assert!(matches!(Wire::take(&bytes), Some(Wire::Later(Later { value: 0x0102030405060708, .. }))));
    assert_eq!(Grant::for_wire(&Wire::Earlier(Earlier { op: 3, value: 0 })), Grant::First);
    assert_eq!(Grant::for_wire(&Wire::Other(Other { op: 5, value: 0 })), Grant::Second);
    assert_eq!(REGISTRY.iter().map(|entry| entry.name).collect::<Vec<_>>(), [
        "test.interface.v1/channel/response",
        "test.interface.v1/grant/second",
        "test.interface.v1/grant/first",
    ]);
    assert_eq!(RENAMED, env::Mark::of("test.interface.v1/channel/response"));
    assert!(wire::marks::conflict(&[REGISTRY]).is_none());
}
