use env::PieToken;
use env::wire::Span as _;
use pci_api::{
    BACK, BAR_MARK, Call, DISABLE, ENABLE, ENTRY, ENTRY_NAME, ENTRY_PATH, Fail, PLACE, Place,
    RELEASE, Said, Toggle, Wire, code_to_fail, fail_to_code,
};
use wire::Message;

#[test]
fn requests_all_variants_golden_and_bounds() {
    let place = Place {
        op: PLACE,
        serial: 0x0807060504030201,
        requester: 0x0c0b0a09,
        bar: 0x0d,
        back: PieToken::mint(0x1514131211100f0e),
    };
    let place_golden: [u8; 22] = [
        0x01, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15,
    ];
    let mut place_bytes = [0xa5u8; Place::LEN];
    let place_len = place.store_at(&mut place_bytes, 0).unwrap();
    assert_eq!(place_len, 22);
    assert_eq!(&place_bytes[..place_len], &place_golden);
    assert_eq!(Place::LEN, 24);

    let disable = Toggle {
        op: DISABLE,
        serial: 0x0807060504030201,
        requester: 0x0c0b0a09,
        back: PieToken::mint(0x14131211100f0e0d),
    };
    let toggle_golden: [u8; 21] = [
        0x03, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f, 0x10, 0x11, 0x12, 0x13, 0x14,
    ];
    let mut toggle_bytes = [0xa5u8; Toggle::LEN];
    let toggle_len = disable.store_at(&mut toggle_bytes, 0).unwrap();
    assert_eq!(toggle_len, 21);
    assert_eq!(&toggle_bytes[..toggle_len], &toggle_golden);
    assert_eq!(Toggle::LEN, 24);

    let enable = Toggle {
        op: ENABLE,
        ..disable
    };
    let enable_golden: [u8; 21] = [
        0x02, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f, 0x10, 0x11, 0x12, 0x13, 0x14,
    ];
    let mut enable_bytes = [0xa5u8; Toggle::LEN];
    let enable_len = enable.store_at(&mut enable_bytes, 0).unwrap();
    assert_eq!(enable_len, 21);
    assert_eq!(&enable_bytes[..enable_len], &enable_golden);

    let release = Toggle {
        op: RELEASE,
        ..disable
    };
    let release_golden: [u8; 21] = [
        0x04, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f, 0x10, 0x11, 0x12, 0x13, 0x14,
    ];
    let mut release_bytes = [0xa5u8; Toggle::LEN];
    let release_len = release.store_at(&mut release_bytes, 0).unwrap();
    assert_eq!(release_len, 21);
    assert_eq!(&release_bytes[..release_len], &release_golden);

    assert_eq!(Wire::LEN, 24);
    let requests = [
        (Wire::Place(place), &place_golden[..]),
        (Wire::Enable(enable), &enable_golden[..]),
        (Wire::Disable(disable), &toggle_golden[..]),
        (Wire::Release(release), &release_golden[..]),
    ];
    for (request, golden) in requests {
        let mut out = [0u8; Wire::LEN];
        let written = request.store(&mut out).unwrap();
        assert_eq!(&out[..written], golden);
        assert_eq!(Wire::take(&out[..written]), Some(request));
        assert_eq!(Wire::take(&out[..written - 1]), None);
        let mut extra = [0u8; Wire::LEN];
        extra[..written].copy_from_slice(&out[..written]);
        extra[written] = 0xff;
        assert_eq!(Wire::take(&extra[..written + 1]), None);
    }
    assert_eq!(Wire::take(&[]), None);
    assert_eq!(Wire::take(&[0x00]), None);
    assert_eq!(Wire::take(&[0x05]), None);

    let mut mismatch = [0u8; Wire::LEN];
    assert_eq!(
        Wire::Place(Place {
            op: ENABLE,
            ..place
        })
        .store(&mut mismatch),
        None
    );
    assert_eq!(
        Wire::Enable(Toggle {
            op: DISABLE,
            ..disable
        })
        .store(&mut mismatch),
        None
    );
    assert_eq!(
        Wire::Disable(Toggle {
            op: ENABLE,
            ..disable
        })
        .store(&mut mismatch),
        None
    );
    assert_eq!(
        Wire::Release(Toggle {
            op: PLACE,
            ..disable
        })
        .store(&mut mismatch),
        None
    );
}

#[test]
fn reply_golden_and_malformed_strict() {
    let said = Said {
        status: 5,
        pages: 0x04030201,
        bar: PieToken::mint(0x0c0b0a0908070605),
    };
    let said_golden: [u8; 13] = [
        0x05, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
    ];
    let mut said_bytes = [0xa5u8; Said::LEN];
    let said_len = said.store(&mut said_bytes).unwrap();
    assert_eq!(said_len, 13);
    assert_eq!(&said_bytes[..said_len], &said_golden);
    assert_eq!(Said::LEN, 16);
    assert_eq!(Said::fetch(&said_golden), Some(said));

    assert_eq!(Said::fetch(&said_golden[..12]), None);
    assert_eq!(Said::fetch(&[]), None);
    assert_eq!(Said::fetch(&[0x05]), None);

    let mut extra = [0u8; 14];
    extra[..13].copy_from_slice(&said_golden);
    extra[13] = 0xff;
    assert_eq!(Said::fetch(&extra), None);

    let mut padded = [0u8; 16];
    padded[..13].copy_from_slice(&said_golden);
    assert_eq!(Said::fetch(&padded), None);

    assert_eq!(Said::NONE, Said::of(0));
    assert_eq!(
        Said::of(5),
        Said {
            status: 5,
            ..Said::NONE
        }
    );
}

#[test]
fn contract_back_reply_token_and_marks() {
    assert_eq!(BACK, env::Mark::of("pci-back"));
    assert_eq!(Call::BACK, BACK);
    assert_eq!(BAR_MARK, env::Mark::of("pci/recovery-bar"));
    assert_eq!(ENTRY, pci_declaration::marks::ENTRY);
    assert_eq!(ENTRY_NAME, "entry");
    assert_eq!(ENTRY_PATH, "pci/entry");
    assert_eq!((PLACE, ENABLE, DISABLE, RELEASE), (1, 2, 3, 4));

    assert_eq!(fail_to_code(None), wire::OK);
    assert_eq!(fail_to_code(Some(Fail::Unknown)), 1);
    assert_eq!(fail_to_code(Some(Fail::Conflict)), 2);
    assert_eq!(fail_to_code(Some(Fail::Denied)), 3);
    assert_eq!(fail_to_code(Some(Fail::Unavailable)), 4);
    assert_eq!(fail_to_code(Some(Fail::Bad)), 5);
    assert_eq!(code_to_fail(wire::OK), None);
    assert_eq!(code_to_fail(1), Some(Fail::Unknown));
    assert_eq!(code_to_fail(5), Some(Fail::Bad));
    assert_eq!(code_to_fail(6), Some(Fail::Bad));
    assert_eq!(code_to_fail(0xff), Some(Fail::Bad));

    let place_back = PieToken::mint(0x0102030405060708);
    let toggle_back = PieToken::mint(0x1112131415161718);
    let place = Place {
        op: PLACE,
        serial: 0x0807060504030201,
        requester: 0x0c0b0a09,
        bar: 0x0d,
        back: place_back,
    };
    let toggle = Toggle {
        op: ENABLE,
        serial: 0x0807060504030201,
        requester: 0x0c0b0a09,
        back: toggle_back,
    };
    assert_eq!(Call::back(&Wire::Place(place)), place_back);
    assert_eq!(Call::back(&Wire::Enable(toggle)), toggle_back);
    assert_eq!(
        Call::back(&Wire::Disable(Toggle {
            op: DISABLE,
            ..toggle
        })),
        toggle_back
    );
    assert_eq!(
        Call::back(&Wire::Release(Toggle {
            op: RELEASE,
            ..toggle
        })),
        toggle_back
    );
}
