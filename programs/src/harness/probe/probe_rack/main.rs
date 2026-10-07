#![no_std]
#![no_main]

//! probe-rack — **一具架的队列语义与唤醒协议**（单域，确定；不碰树、不碰设备）。
//!
//! 这一台量的就是"**页上那一位**"：架上没有第二枚 Pie，写端落一格响的那一位、读端等/应的
//! 那一位，都是那枚页自己的。
//!
//! # 判据（每一条失败都当场塌——整机那一格判的是有没有 `EXIT_PANIC`）
//! 1. **`Mode::Newest` 的界**：连落 `CAP` 条全 `Ok`；第 `CAP+1` 条答 `Full`（`dropped=1`），
//!    **不是"第 16 条之后永久满"**；`pending` 是架上的真实深度。
//! 2. **顺序与内容**：逐条读回，与落下的那一条逐字节对得上。
//! 3. **不空转**：读干之后 `wait(AtMost(5))` 答 `Ok(false)`——修之前那一位一直亮着，它会当场答 `true`。
//! 4. **唤醒协议**：再落一条 ⇒ `wait` 答 `Ok(true)`，`recv(POLL)` 读到它。
//! 5. **`Mode::Oldest` 的账**：连落 `CAP+3` 条全 `Ok`，`lost=3`、`pending=CAP`；
//!    读端能继续读、读到的号**严格递增**，且**看见了跳号**（`skipped ≥ 3`）。
//!
//! 读数一行（`debug!` 在 release 是空操作，故这一行走 `debug::put`）。

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;
use programs::driver::uart::core::frame::Bytes;
use ipc::rack::{CAP, Mode, Rack, SendFail};
use ipc::session::Session;
use protocol::system::operator::client as operator;
use env::unit;

/// 等板 / 等树那一趟的额度（毫秒）
const MS: usize = 1000;

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）
const OK_NOTE: &str = "probe-rack: newest_full=1 quiet=false";

#[programs::entry]
fn main() -> Report<'static> {
    // 会话保持到探针结束，供 System 识别已就绪的树连接。
    let Ok(_session) = Session::open(unit::sire(), operator::BERTH, Wait::AtMost(MS)) else {
        panic!("probe-rack: 树那条路开不出来（装配者等的那一枚 LINK）");
    };
    let (dropped, _) = newest();
    let (lost, skipped, got) = oldest();
    protocol::debug::put(&alloc::format!(
        "probe-rack: newest_dropped={dropped} newest_pending={CAP} oldest_lost={lost} \
         oldest_skipped={skipped} oldest_got={got} quiet=false"
    ));
    Report::note(env::EXIT_OK, OK_NOTE)
}

/// 判据 1–4：`Newest` 那一具。返 `(dropped, skipped)`。
fn newest() -> (u64, u64) {
    let rack = open(Mode::Newest);
    let mut w = rack.writer();
    let mut r = rack.reader();
    for i in 0..CAP {
        assert!(
            w.send(&payload(i)).is_ok(),
            "probe-rack: newest 第 {i} 条该进得去"
        );
    }
    assert!(
        matches!(w.send(&payload(CAP)), Err(SendFail::Full)),
        "probe-rack: 第 CAP+1 条该答 Full（满了那一档不是永久满）"
    );
    assert_eq!(w.dropped(), 1, "probe-rack: dropped 该记一枚");
    assert_eq!(w.pending(), CAP as u64, "probe-rack: pending 该是真实深度");
    for i in 0..CAP {
        let one = match r.recv(Wait::POLL) {
            Ok(one) => one,
            Err(fail) => panic!("probe-rack: 读第 {i} 条失败：{fail:?}"),
        };
        assert_eq!(
            one.bytes(),
            payload(i).bytes(),
            "probe-rack: 读到的第 {i} 条不是落下的那一条"
        );
    }
    // 读干这一趟会把铃应掉（`recv` 那三拍），故下一手等铃该是"没有"。
    assert!(
        r.recv(Wait::POLL).is_err(),
        "probe-rack: 架空了却还读到一条"
    );
    match r.wait(Wait::AtMost(5)) {
        Ok(false) => {}
        other => panic!("probe-rack: 读干之后等铃该是 Ok(false)（不空转），实测 {other:?}"),
    }
    // 唤醒协议：再落一条 ⇒ 铃该响，且读到它。
    assert!(w.send(&payload(99)).is_ok(), "probe-rack: 唤醒那一落");
    match r.wait(Wait::AtMost(50)) {
        Ok(true) => {}
        other => panic!("probe-rack: 落了新的一条，等铃该是 Ok(true)，实测 {other:?}"),
    }
    let one = match r.recv(Wait::POLL) {
        Ok(one) => one,
        Err(fail) => panic!("probe-rack: 唤醒之后读不到：{fail:?}"),
    };
    assert_eq!(
        one.bytes(),
        payload(99).bytes(),
        "probe-rack: 唤醒之后读到的那一条不对"
    );
    (w.dropped(), r.skipped())
}

/// 判据 5：`Oldest` 那一具。返 `(lost, skipped, got)`。
fn oldest() -> (u64, u64, usize) {
    let rack = open(Mode::Oldest);
    let mut w = rack.writer();
    let mut r = rack.reader();
    for i in 0..CAP + 3 {
        assert!(
            w.send(&payload(i)).is_ok(),
            "probe-rack: oldest 第 {i} 条该进得去（Oldest 不答 Full）"
        );
    }
    assert_eq!(w.lost(), 3, "probe-rack: 顶掉的未读格该是 3");
    assert_eq!(w.pending(), CAP as u64, "probe-rack: 架上仍是满的 CAP 格");
    let mut got = 0usize;
    let mut last: Option<u32> = None;
    while let Ok(one) = r.recv(Wait::POLL) {
        let bytes = one.bytes();
        let at = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        if let Some(prev) = last {
            assert!(at > prev, "probe-rack: 读到的号没递增（{prev} → {at}）");
        }
        last = Some(at);
        got += 1;
    }
    assert!(
        got > 0 && got < CAP,
        "probe-rack: Oldest 那一档该读到几条、但不是全部（实测 {got} 条）"
    );
    assert!(
        r.skipped() >= 3,
        "probe-rack: 读端该看见跳号（写端至少丢了 3 枚），实测 {}",
        r.skipped()
    );
    (w.lost(), r.skipped(), got)
}

/// 开一具架（起手没材料 ⇒ 这一台当场塌）。
fn open(mode: Mode) -> Rack<Bytes> {
    match Rack::<Bytes>::open(mode) {
        Ok(rack) => rack,
        Err(fail) => panic!("probe-rack: 开不出一具架：{fail:?}"),
    }
}

/// 那一条载荷：与另外两台同一手（`probe::rack`）。
fn payload(i: usize) -> Bytes {
    programs::harness::probe::rack::payload(i)
}
