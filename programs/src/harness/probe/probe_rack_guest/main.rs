#![no_std]
#![no_main]

//! probe-rack-guest — **对端那一侧**：用**产品同一条**客人面（`client::find`）把**两枚号**取回来，
//! 先读完 A，再写满 B。
//!
//! # 判据（失败即 `panic!`）
//! 1. **两枚号取齐**：`Reader::from_token` / `Writer::from_token` 各把对端那一枚**页**映进来
//!    ——页上那一位就是铃，故一枚号同时给出字节与"有事"（共映射那一半在全树此前零用家）。
//! 2. **读到的就是铺场落下的**：`CAP` 条、逐条逐字节对得上、号严格递增。
//! 3. **次序**：先读完 A 再写 B——铺场那一台拿"B 满"当"客人读完了"的凭据。
//!
//! 读数一行（`debug!` 在 release 是空操作，故这一行走 `debug::put`）。

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;
use programs::driver::uart::client;
use programs::harness::probe::rack as rig;
use protocol::communication::rack::Mode;
use protocol::communication::session::Session;
use protocol::system::operator::client as operator;
use protocol::system::operator::Face;
use runtime::env::unit;

/// 等板 / 等树那一趟的额度（毫秒）
const MS: usize = 1000;

/// 等那几条的额度（毫秒）：铺场是"先写满 A 才响 `Ready`"，故这一手不会白等
const WAIT_MS: usize = 3_000;

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）
const OK_NOTE: &str = "probe-rack-guest: read=16 wrote=16";

#[programs::entry]
fn main() -> Report<'static> {
    let Ok(session) = Session::open(unit::sire(), operator::BERTH, Wait::AtMost(MS)) else {
        panic!("probe-rack-guest: 树那条路开不出来");
    };
    let tree = Face::of(session);
    let road = match rig::road() {
        Some(road) => road,
        None => panic!("probe-rack-guest: 试验场那条路拼不出来"),
    };
    // **产品同一条**客人面：四个名字 → 两端句柄（本台不去碰 `Rack::open`，也不碰页布局）。
    let mut console = match client::find(&tree, &road, Mode::Oldest, Wait::AtMost(MS)) {
        Some(console) => console,
        None => panic!("probe-rack-guest: 两枚号取不齐"),
    };
    // 先读 A：铺场已经写满，故这几条该当场就有。
    for i in 0..rig::count() {
        let one = match console.rx.recv(Wait::AtMost(WAIT_MS)) {
            Ok(one) => one,
            Err(fail) => panic!("probe-rack-guest: 读 A 的第 {i} 条失败：{fail:?}"),
        };
        assert_eq!(
            one.bytes(),
            rig::payload(i).bytes(),
            "probe-rack-guest: A 上第 {i} 条不是铺场落的那一条"
        );
    }
    // 再写 B（**次序是契约**：写满 B 就是"我已经读完 A"的凭据）。
    for i in 0..rig::count() {
        assert!(
            console.tx.send(&rig::payload(i)).is_ok(),
            "probe-rack-guest: B 第 {i} 条没落进去"
        );
    }
    protocol::debug::put(&alloc::format!(
        "probe-rack-guest: read={} wrote={}",
        rig::count(),
        rig::count()
    ));
    Report::note(env::EXIT_OK, OK_NOTE)
}
