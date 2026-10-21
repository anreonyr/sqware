/// 新协议那一侧的等价编解码回归：旧私有启动收据（`Completed` 两字布局与
/// bool 非法值拒绝）已随收据本身删除，这里对**现在真的在用的** DMA 协调帧
/// 核同两条口径：线上布局钉死、多余字节与未知动作码一律拒。
///
/// 帧本身是 `env::Frame`（`Span`），`Message` 实现长在 `DmaWire` 上；
/// 故编解码都走 `DmaWire`（与线上同一层）。

use env::PieToken;
use hub_api::{
    BindingState, CloseBinding, DmaClaim, DmaSaid, DmaWire, Inspect, Settle, DMA_CLAIM,
    DMA_CLOSE, DMA_INSPECT, DMA_SETTLE,
};
use wire::Message;

fn token(raw: u64) -> PieToken {
    PieToken::from_bytes(&raw.to_le_bytes()).unwrap()
}

#[test]
fn dma_claim_keeps_its_exact_layout_and_rejects_trailing_bytes() {
    let back = token(0x0102_0304_0506_0708);
    let sensor = token(0x1112_1314_1516_1718);
    let wire = DmaWire::Claim(DmaClaim {
        op: DMA_CLAIM,
        id: 0x1122_3344_5566_7788,
        sensor,
        back,
    });
    let mut bytes = DmaWire::EMPTY;
    let len = wire.store(&mut bytes).unwrap();
    assert_eq!(len, 1 + 8 + 8 + 8);
    assert_eq!(&bytes[..len], &[
        2, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, 0x18, 0x17, 0x16, 0x15, 0x14,
        0x13, 0x12, 0x11, 8, 7, 6, 5, 4, 3, 2, 1,
    ]);
    assert_eq!(DmaWire::fetch(&bytes[..len]), Some(wire));
    assert!(DmaWire::fetch(&bytes[..len - 1]).is_none());
    let mut extended = bytes.to_vec();
    extended.push(0);
    assert!(DmaWire::fetch(&extended).is_none());
}

#[test]
fn unknown_action_codes_are_refused() {
    let mut bytes = [0u8; 32];
    bytes[0] = 0x7F;
    assert!(DmaWire::fetch(&bytes).is_none());
}

#[test]
fn close_inspect_settle_round_trip() {
    let back = token(7);
    for wire in [
        DmaWire::Close(CloseBinding {
            op: DMA_CLOSE,
            serial: 3,
            reason: 2,
            back,
        }),
        DmaWire::Inspect(Inspect {
            op: DMA_INSPECT,
            serial: 3,
            back,
        }),
        DmaWire::Settle(Settle {
            op: DMA_SETTLE,
            back,
        }),
    ] {
        let mut bytes = DmaWire::EMPTY;
        let len = wire.store(&mut bytes).unwrap();
        assert_eq!(DmaWire::fetch(&bytes[..len]), Some(wire));
    }
}

#[test]
fn said_keeps_its_layout_and_never_carries_a_device_list() {
    let said = DmaSaid::bound(9, BindingState::Preparing);
    let mut bytes = DmaSaid::EMPTY;
    let len = said.store(&mut bytes).unwrap();
    assert_eq!(len, 1 + 1 + 1 + 1 + 1 + 1 + 8);
    assert_eq!(DmaSaid::fetch(&bytes[..len]), Some(said));
    assert!(DmaSaid::fetch(&bytes[..len - 1]).is_none());
}
