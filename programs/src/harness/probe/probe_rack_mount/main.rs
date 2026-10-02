#![no_std]
#![no_main]

//! probe-rack-mount — **跨域共映射那一侧**（铺场）：把两具架当**两枚门牌**落到树上
//! （每枚后面是一具完整的架：页上那一位即铃），把 A 写满，再响 `Ready` 让客人起步。
//!
//! # 判据（失败即 `panic!`）
//! 1. **两枚砖都落成**：`land` / `find` / 名字三格逐枚对得上——**页当门牌**这件事在这之前
//!    全树零用家（`find` 的存活探针从前只认孔）。
//! 2. **A 落满 `CAP` 条**（不绕环 ⇒ 客人该读到几条是确定的）。
//! 3. **等到 B 的 `CAP` 条**：客人是"先读完 A、再写 B"的（见客人那一台的头注），
//!    故这一台见到 B 满 = 客人已经读完 A——**这一条次序就是两台之间唯一的同步**。
//! 4. **树会剔死那一格**：把 A 那一枚页封印之后，`find` 该答 `Dead`（树当场把那一格摘掉）。
//!    ——这一条是 `PieCall::Alive` 落点的正证：那一手从前写成 `locate + usable`（只看"有没有
//!    交出去"），页封印之后它会照答"在"，这一格就永远剔不掉。
//!
//! 读数一行（`debug!` 在 release 是空操作，故这一行走 `debug::put`）。

extern crate alloc;
extern crate programs;

use alloc::vec::Vec;

use env::{Mark, Wait};
use programs::Report;
use programs::driver::uart::core::frame::Bytes;
use programs::harness::probe::rack as rig;
use programs::service::operator::bridge;
use programs::service::operator::bridge::Landed;
use protocol::communication::rack::{Mode, Rack};
use protocol::communication::session::{Session, establish};
use protocol::service::operator::client as operator;
use protocol::service::operator::client::{Face, Mine};
use protocol::service::operator::{Fail, Grant, Permit};
use runtime::env::mail;
use runtime::env::unit as utask;

/// 等板 / 等树那一趟的额度（毫秒）
const MS: usize = 1000;

/// 等客人那几条的额度（毫秒）：装配窗口 ＋ 它那一趟读写，给足
const WAIT_MS: usize = 3_000;

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）
const OK_NOTE: &str = "probe-rack-mount: landed=2 sealed=pruned";

#[programs::entry]
fn main() -> Report<'static> {
    let Ok(session) = Session::open(utask::sire(), operator::BERTH, Wait::AtMost(MS)) else {
        panic!("probe-rack-mount: 树那条路开不出来");
    };
    let tree = Face::of(session);
    // A：铺场写、客人读（与产品那两条同形）；B：客人写、铺场读。
    let a = open(Mode::Oldest);
    let b = open(Mode::Oldest);
    let plated = land(&tree, &a, &b);

    // A 先写满：客人一取号就该有东西可读（**不绕环**，故"读到几条"是确定的）。
    let mut aw = a.writer();
    for i in 0..rig::count() {
        assert!(
            aw.send(rig::payload(i)).is_ok(),
            "probe-rack-mount: A 第 {i} 条没落进去"
        );
    }

    // **响 `Ready`**：客人的装配声明指着这一台，故它等这一声才起步。
    let _ = establish::endpoint(utask::sire(), Mark::of(programs::unit::READY), Wait::POLL);

    // 等 B：见到 CAP 条 = 客人已经读完 A（它那一边的次序）。
    let mut br = b.reader();
    for i in 0..rig::count() {
        let one = match br.recv(Wait::AtMost(WAIT_MS)) {
            Ok(one) => one,
            Err(fail) => panic!("probe-rack-mount: 等 B 的第 {i} 条失败：{fail:?}"),
        };
        assert_eq!(
            one.bytes(),
            rig::payload(i).bytes(),
            "probe-rack-mount: B 上第 {i} 条不是客人落的那一条"
        );
    }

    // **判据 4**：封印 A 那一枚页 ⇒ 树上那一格该被剔掉（`find` 答 `Dead`）。
    // `find` 自成一位（那一手会转移权柄）⇒ 要 `Grant::Find` 那一柄。
    assert!(
        mail::seal(a.ship()).is_ok(),
        "probe-rack-mount: 封印自己那一枚页失败"
    );
    let rein = tree.rein(Grant::Find);
    match rein.find(plated[0].plate, Wait::AtMost(MS)) {
        Err(Fail::Dead) => {}
        other => panic!("probe-rack-mount: 封印之后那一格该答 Dead，实测 {other:?}"),
    }
    protocol::debug::put(&alloc::format!(
        "probe-rack-mount: landed={} wrote={} read={} sealed=pruned",
        plated.len(),
        rig::count(),
        rig::count()
    ));
    Report::note(env::EXIT_OK, OK_NOTE)
}

/// 开一具架（起手没材料 ⇒ 这一台当场塌）。
fn open(mode: Mode) -> Rack<Bytes> {
    match Rack::<Bytes>::open(mode) {
        Ok(rack) => rack,
        Err(fail) => panic!("probe-rack-mount: 开不出一具架：{fail:?}"),
    }
}

/// 把那**两枚号**落到树上的试验场里（`Mine::No` ＋ `Permit::Unset`：谁都能查、谁都能取，
/// 与那两条产品门牌同一条公开口径）。返落成的那两格（判据 4 要那一号）。
fn land(tree: &Face, a: &Rack<Bytes>, b: &Rack<Bytes>) -> Vec<Landed> {
    let road = match rig::road() {
        Some(road) => road,
        None => panic!("probe-rack-mount: 试验场那条路拼不出来"),
    };
    let faces = rig::faces(a, b);
    let plated = bridge::land(
        tree,
        rig::ROAD,
        &road,
        Mine::No,
        Permit::Unset,
        &faces,
        Wait::AtMost(MS),
    );
    assert_eq!(plated.len(), 2, "probe-rack-mount: 两枚砖没落齐");
    for (one, (want, _)) in plated.iter().zip(faces.iter()) {
        assert!(one.land.is_ok(), "probe-rack-mount: 落 {want} 失败");
        assert!(one.find.is_ok(), "probe-rack-mount: 查回 {want} 失败");
        assert_eq!(
            one.named.as_ref().map(|name| name.as_str()),
            Some(*want),
            "probe-rack-mount: {want} 那一格的名字对不上"
        );
    }
    plated
}
