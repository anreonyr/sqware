use crate::{PieToken, TaskId};
use crate::client::{Observe, Supply, Wire, OBSERVE, SUPPLY};
use crate::command::Command;
use wire::Message;

#[test]
fn binding_client_frames_preserve_full_identity_and_reject_partial_or_extra_bytes() {
    let requests = [
        Wire::Observe(Observe { op: OBSERVE, hub: TaskId::new(7), serial: u64::MAX,
            back: PieToken::mint(11) }),
        Wire::Supply(Supply { op: SUPPLY, hub: TaskId::new(7), serial: u64::MAX,
            source: PieToken::mint(13), access: 3, back: PieToken::mint(11) }),
    ];
    for (request, expected) in requests.into_iter().zip([25, 37]) {
        let mut bytes = Wire::EMPTY;
        let len = request.store(&mut bytes).unwrap();
        assert_eq!(len, expected);
        assert_eq!(&bytes[1..9], &7u64.to_le_bytes());
        assert_eq!(&bytes[9..17], &u64::MAX.to_le_bytes());
        assert_eq!(Wire::fetch(&bytes[..len]), Some(request));
        for end in 0..len { assert!(Wire::fetch(&bytes[..end]).is_none()); }
        assert!(Wire::fetch(&bytes[..len + 1]).is_none());
        bytes[0] = 0xff;
        assert!(Wire::fetch(&bytes[..len]).is_none());
    }
    let mut bytes = Wire::EMPTY;
    assert!(Wire::Observe(Observe { op: SUPPLY, hub: TaskId::new(7), serial: 9,
        back: PieToken::mint(11) }).store(&mut bytes).is_none());
}

#[test]
fn window_command_delivers_the_exact_target_local_token() {
    for command in [Command::Window(PieToken::mint(0x99)), Command::Ring, Command::Close,
        Command::Exit, Command::Probe, Command::Submit] {
        let mut bytes = [0; Command::LEN];
        let len = command.store(&mut bytes);
        assert_eq!(Command::fetch(&bytes[..len]), Some(command));
        assert!(Command::fetch(&bytes[..len - 1]).is_none());
        let mut extra = bytes[..len].to_vec();
        extra.push(0);
        assert!(Command::fetch(&extra).is_none());
    }
    let mut bytes = [0; Command::LEN];
    bytes[0] = 1;
    assert!(Command::fetch(&bytes).is_none(), "an absent receipt is not a window");
}

#[test]
fn coordinates_keep_bar_receipt_bar_size_and_dma_range_distinct() {
    let coords = crate::coords::Coords { bar_pages: 4, common: 0, notify: 4096,
        multiplier: 4, device: 12288, iova: 1 << 33, bar: PieToken::mint(0x99), bytes: 6 * 4096 };
    let mut bytes = crate::coords::Coords::EMPTY;
    let len = coords.store(&mut bytes).unwrap();
    assert_eq!(len, 44);
    assert_eq!(&bytes[28..36], &0x99u64.to_le_bytes());
    assert_eq!(&bytes[36..44], &(6u64 * 4096).to_le_bytes());
    assert_eq!(crate::coords::Coords::fetch(&bytes[..len]), Some(coords));
    assert!(crate::coords::Coords::fetch(&bytes[..len - 1]).is_none());
    assert!(crate::coords::Coords::fetch(&bytes[..len + 1]).is_none());
}
