use pipe_api::{Direction, Endpoint};
use shell_api::{Binding, Catalogue, Image, Launch};
fn token(n: u64) -> abi::PieToken {
    abi::PieToken::from_bytes(&n.to_le_bytes()).unwrap()
}
#[test]
fn launch_round_trip_and_all_truncations_are_rejected() {
    let launch = Launch {
        args: vec!["中".into(), "a\0b".into()],
        ports: vec![Binding {
            name: "records".into(),
            endpoint: Endpoint {
                id: 42,
                capacity: 17,
                seed: token(21),
                life: token(22),
                direction: Direction::Write,
            },
        }],
    };
    let bytes = launch.encode().unwrap();
    let decoded = Launch::decode(&bytes).unwrap();
    assert_eq!(decoded.args, launch.args);
    assert_eq!(decoded.ports[0].endpoint.seed, token(21));
    assert_eq!(decoded.ports[0].endpoint.life, token(22));
    for length in 0..bytes.len() {
        assert!(Launch::decode(&bytes[..length]).is_err(), "{length}");
    }
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(Launch::decode(&extra).is_err());
    let mut version = bytes.clone();
    version[8] = 9;
    assert!(Launch::decode(&version).is_err());
}
#[test]
fn catalogue_contract_rejects_duplicate_images_ports_and_invalid_utf8() {
    let catalogue = Catalogue {
        images: vec![Image {
            name: "emit".into(),
            seed: token(12),
            length: 100,
            ports: vec![("records".into(), Direction::Write)],
        }],
    };
    let bytes = catalogue.encode().unwrap();
    let decoded = Catalogue::decode(&bytes).unwrap();
    assert_eq!(decoded.images[0].ports[0].0, "records");
    for length in 0..bytes.len() {
        assert!(Catalogue::decode(&bytes[..length]).is_err());
    }
    let mut invalid = bytes;
    invalid[32] = 255;
    assert!(Catalogue::decode(&invalid).is_err());
    let duplicate = Catalogue {
        images: vec![Image {
            name: "emit".into(),
            seed: token(12),
            length: 100,
            ports: vec![
                ("records".into(), Direction::Write),
                ("records".into(), Direction::Read),
            ],
        }],
    };
    assert!(Catalogue::decode(&duplicate.encode().unwrap()).is_err());
}
