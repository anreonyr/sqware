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

use ::resource::raw::inspect;
use env::Wait;
use env::pie;
use env::unit;
use ipc::rack::{Mode, Rack};
use ipc::session::{Session, establish};
use programs::Report;
use programs::driver::uart::core::frame::Bytes;
use programs::harness::probe::rack as rig;
use system_api::control::Scope;
use system_api::control::Target;
use system_api::operator::Fail;
use system_api::operator::Permit;
use system_client::control::publication::Client;
use system_client::operator;
use system_client::operator::Face;

/// 等板 / 等树那一趟的额度（毫秒）
const MS: usize = 1000;

/// 等客人那几条的额度（毫秒）：装配窗口 ＋ 它那一趟读写，给足
const WAIT_MS: usize = 3_000;

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）
const OK_NOTE: &str = "probe-rack-mount: landed=2 sealed=pruned";

#[programs::entry]
fn main() -> Report<'static> {
    let Ok(session) = Session::open(unit::sire(), operator::BERTH, Wait::AtMost(MS)) else {
        panic!("probe-rack-mount: 树那条路开不出来");
    };
    let tree = Face::of(session);
    // A：铺场写、客人读（与产品那两条同形）；B：客人写、铺场读。
    let a = open(Mode::Oldest);
    let b = open(Mode::Oldest);
    let plated = land(&tree, &a, &b);
    let client = Client::injected().unwrap();
    let target = |name: &str| Target::Service {
        scope: Scope(4),
        group: rig::ROAD.into(),
        name: name.into(),
    };
    assert_eq!(
        client.publish(target("rx"), a.ship(), Permit::Public, Wait::AtMost(MS)),
        Ok(plated[0])
    );
    assert_eq!(
        client.publish(target("rx"), b.ship(), Permit::Public, Wait::AtMost(MS)),
        Err(Fail::Denied)
    );

    // A 先写满：客人一取号就该有东西可读（**不绕环**，故"读到几条"是确定的）。
    let mut aw = a.writer();
    for i in 0..rig::count() {
        assert!(
            aw.send(&rig::payload(i)).is_ok(),
            "probe-rack-mount: A 第 {i} 条没落进去"
        );
    }

    // **响 `Ready`**：客人的装配声明指着这一台，故它等这一声才起步。
    let _ = establish::endpoint(unit::sire(), programs::unit::READY_MARK, Wait::POLL);

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

    let retained = tree
        .tile(
            &rig::road().unwrap().try_join("tx").unwrap(),
            Wait::AtMost(MS),
        )
        .unwrap()
        .token(Wait::AtMost(MS))
        .unwrap();
    let mut reader = ipc::rack::Reader::<Bytes>::from_raw(retained).unwrap();
    client.unpublish(target("tx"), Wait::AtMost(MS)).unwrap();
    assert_eq!(
        tree.root()
            .tile(
                &rig::road().unwrap().try_join("tx").unwrap(),
                Wait::AtMost(MS)
            )
            .map(|_| ()),
        Err(Fail::Unknown)
    );
    b.writer().send(&rig::payload(99)).unwrap();
    assert_eq!(
        reader.recv(Wait::AtMost(MS)).unwrap().bytes(),
        rig::payload(99).bytes()
    );
    assert_eq!(inspect(retained).unwrap().owner, unit::self_id());
    programs::debug::put(
        "probe-rack-mount: page publication duplicate/conflict and unpublish preserves delivered mapping",
    );

    // **判据 4**：封印 A 那一枚页 ⇒ 树上那一格该被剔掉（`find` 答 `Dead`）。
    assert!(
        pie::seal(a.ship()).is_ok(),
        "probe-rack-mount: 封印自己那一枚页失败"
    );
    match tree.find(plated[0], Wait::AtMost(MS)) {
        Err(Fail::Dead) => {}
        other => panic!("probe-rack-mount: 封印之后那一格该答 Dead，实测 {other:?}"),
    }
    programs::debug::put(&alloc::format!(
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

/// 把那**两枚号**落到树上的试验场里（`Mine::No` ＋ `Permit::Public`：谁都能查、谁都能取，
/// 与那两条产品门牌同一条公开口径）。返落成的那两格（判据 4 要那一号）。
fn land(tree: &Face, a: &Rack<Bytes>, b: &Rack<Bytes>) -> Vec<system_api::operator::EntryId> {
    let road = rig::road().expect("probe-rack-mount: road");
    let publisher = Client::injected().expect("probe-rack-mount: publication entry");
    let mut mounts = Vec::new();
    for (name, entry) in rig::faces(a, b) {
        let target = Target::Service {
            scope: Scope(4),
            group: rig::ROAD.into(),
            name: name.into(),
        };
        let mount = publisher
            .publish(target, entry, Permit::Public, Wait::AtMost(MS))
            .expect("probe-rack-mount: publication");
        let full = road.try_join(name).unwrap();
        tree.tile(&full, Wait::AtMost(MS))
            .unwrap()
            .token(Wait::AtMost(MS))
            .unwrap();
        assert_eq!(tree.root().name(mount, Wait::AtMost(MS)).unwrap(), name);
        mounts.push(mount);
    }
    assert_eq!(mounts.len(), 2);
    mounts
}
