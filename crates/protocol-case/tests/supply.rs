//! 供单那一门的门（**宿主台**）—— **需求单 / 回单 / 上限**，在宿主上真跑一遍。
//!
//! # 这一台钉的是什么
//!
//! `crates/protocol/src/driver/supply/call.rs`（帧、荷载的类型、上限与那张失败码表）与
//! `crates/protocol/src/driver/supply/core.rs`（五格失败域）——两份都**逐字未改**。
//! 判据写在靶子里（照 `protocol-case` 的 `line` 靶的做法），核心源码一个字不动：
//!
//! ```text
//!   Want / Need   一格荷载：坐标 + 类别 + **两族视图**（对端能做什么 / 这一枚能怎么流动）
//!   settle        类 → 区那一手（装配者按机器那张表补坐标）
//!   class_block   按类名造一格（`class: false` 那一形）
//!   帧长与上限     WANT_LEN / ORDER_CAP / REPLY_CAP：条数越界 ⇒ Full（不 panic）
//!   失败码表       五格 ↔ 线上那一格；`Bad`（这一问读不懂）在表外
//! ```

extern crate alloc;

/// 码表宏（`fail_codes!`）自己一份源——**协议与宿主靶同读这一份**（见那份文件的照实记）。
#[macro_use]
#[path = "../../contract/src/fail_codes.rs"]
mod fail_codes;

/// 失败域（就是 `crates/protocol/src/driver/supply/core.rs` 那一份，逐字未改）。
#[path = "../../protocol/src/driver/supply/core.rs"]
mod core;

/// 那一门（就是 `crates/protocol/src/driver/supply/call.rs` 那一份，逐字未改）。
///
/// **模块名就叫 `call`**：它写的是 `use super::core::Fail;`——宿主靶里把两份放在**同一层**、
/// 名字照旧，那一行才逐字成立。
#[allow(dead_code)]
#[path = "../../protocol/src/driver/supply/call.rs"]
mod call;

use crate::call::{Kind, Need, Want};
use crate::core::Fail;
use env::{Access, Name, Policy, TaskId};
use plan::{Key};

fn name(text: &str) -> Name {
    Name::new(text).expect("名字合法")
}

/// 一格"要一段区"的荷载。
fn want(key: Key) -> Want {
    Want::new(key, Kind::Pole, Access::FETCH, Policy::NONE)
}

#[test]
fn a_want_carries_the_coordinate_the_kind_and_the_two_views() {
    let w = Want::new(
        Key::region(0x1000_0000),
        Kind::Pole,
        Access::FETCH | Access::STORE,
        Policy::VEST,
    );
    assert_eq!(w.key(), Some(Key::region(0x1000_0000)));
    assert_eq!(w.kind(), Some(Kind::Pole));
    assert_eq!(w.access(), Some(Access::FETCH_STORE), "两族那两位按位或");
    assert_eq!(w.policy(), Some(Policy::VEST));

    // **两族分家**：`Access` 里写不进传递族那一位（那是"混族不可表达"那一格）。
    assert_ne!(
        w.access().map(|a| a.bits()),
        w.policy().map(|p| p.bits()),
        "两族是不同的位"
    );

    // 空集：四位都空。`ship` 会就地拒（那一条在别的台，这里只钉类型这一格）。
    let none = Want::new(Key::region(8), Kind::Nole, Access::NONE, Policy::NONE);
    assert_eq!(none.access(), Some(Access::NONE));
    assert_eq!(none.kind(), Some(Kind::Nole));
}

#[test]
fn an_order_frame_round_trips_with_its_count_and_who() {
    let who = TaskId::new(41);
    let wants = [want(Key::region(0x1000)), want(Key::irq())];
    let mut buf = [0u8; crate::call::ORDER_CAP];
    let frame = crate::call::pack_order(&mut buf, who, &wants).expect("装得下");
    assert_eq!(frame.len(), 2 + 8 + 2 * crate::call::WANT_LEN);
    assert_eq!(frame[0], crate::call::OP_SUPPLY);
    assert_eq!(frame[1], 2, "条数那一格");

    let order = crate::call::unpack_order(frame).expect("读得回来");
    assert_eq!(order.who(), who);
    assert_eq!(order.len(), 2);
    assert_eq!(
        order.want(0).map(|w| w.key()),
        Some(Some(Key::region(0x1000)))
    );
    assert_eq!(order.want(1).map(|w| w.key()), Some(Some(Key::irq())));
    assert!(order.want(2).is_none(), "越界的那一条没有");
    assert!(order.want(1).is_some());
}

#[test]
fn an_order_that_is_not_that_shape_is_not_guessed_at() {
    let mut buf = [0u8; crate::call::ORDER_CAP];
    let frame = crate::call::pack_order(&mut buf, TaskId::new(1), &[want(Key::region(8))]).unwrap();
    let good = frame.to_vec();

    // 动作码不对 / 太短 / 条数说谎：三种都是"读不懂"。
    let mut wrong_op = good.clone();
    wrong_op[0] = 99;
    assert!(crate::call::unpack_order(&wrong_op).is_none());
    assert!(
        crate::call::unpack_order(&good[..9]).is_none(),
        "连头都不到"
    );
    assert!(
        crate::call::unpack_order(&good[..good.len() - 1]).is_none(),
        "短一字节"
    );
    let mut liar = good.clone();
    liar[1] = 3; // 说有三条，可帧里只有一条
    assert!(crate::call::unpack_order(&liar).is_none(), "条数说谎");

    // 编的时候：条数越界 / 缓冲不够 ⇒ `None`（调用方按本地失败处理，不是 panic）。
    let many: Vec<Want> = (0..crate::call::WANT_MAX + 1)
        .map(|i| want(Key::region(8 + i as u64)))
        .collect();
    let mut big = [0u8; crate::call::ORDER_CAP + crate::call::WANT_LEN];
    assert!(
        crate::call::pack_order(&mut big, TaskId::new(1), &many).is_none(),
        "条数越界"
    );
    let mut small = [0u8; 16];
    assert!(
        crate::call::pack_order(&mut small, TaskId::new(1), &[want(Key::region(8))]).is_none(),
        "缓冲不够"
    );
}

#[test]
fn a_reply_frame_round_trips_and_refuses_a_ragged_record_block() {
    use plan::{PAIR_LEN, Pair};

    let records = {
        let a = Pair::bytes(Key::region(0x1000), 3);
        let b = Pair::bytes(Key::irq(), 4);
        [a.as_slice(), b.as_slice()].concat()
    };
    let mut buf = [0u8; crate::call::REPLY_CAP];
    let frame = crate::call::pack_reply(&mut buf, crate::call::OK, &records).expect("装得下");
    assert_eq!(frame.len(), 2 + 2 * PAIR_LEN);
    assert_eq!(frame[1], 2, "记录条数那一格");
    // 先拷出来：`frame` 借着那只缓冲，后面还要再编两张。
    let good = frame.to_vec();

    let reply = crate::call::unpack_reply(&good).expect("读得回来");
    assert_eq!(reply.code(), crate::call::OK);
    assert_eq!(reply.records(), &records[..], "记录那一段原样交回");

    // **一条记录是 `PAIR_LEN` 步长**：零头那一块不许编（也不许读）。
    let mut buf2 = [0u8; crate::call::REPLY_CAP];
    assert!(
        crate::call::pack_reply(&mut buf2, crate::call::OK, &records[..PAIR_LEN - 1]).is_none(),
        "零头"
    );
    let mut too_many = alloc::vec![0u8; PAIR_LEN * (crate::call::WANT_MAX + 1)];
    too_many.fill(7);
    let mut buf3 = [0u8; crate::call::REPLY_CAP];
    assert!(
        crate::call::pack_reply(&mut buf3, crate::call::OK, &too_many).is_none(),
        "条数越界"
    );

    // 读的时候长度必须与条数对得上。
    assert!(
        crate::call::unpack_reply(&good[..good.len() - 1]).is_none(),
        "短一字节"
    );
    let mut liar = good.clone();
    liar[1] = 1;
    assert!(crate::call::unpack_reply(&liar).is_none(), "条数说谎");
    assert!(crate::call::unpack_reply(&[]).is_none(), "空帧");
    // 答话是**失败**码时形状照旧（那一格由调用方读）。
    let mut buf4 = [0u8; crate::call::REPLY_CAP];
    let failed = crate::call::pack_reply(&mut buf4, crate::call::DENIED, &[]).unwrap();
    assert_eq!(
        crate::call::unpack_reply(failed).map(|r| (r.code(), r.records().len())),
        Some((crate::call::DENIED, 0))
    );
}

#[test]
fn a_need_settles_into_a_want_through_the_class_name() {
    use crate::call::{At, Need, class_block};

    // 按类名造一格：类名进的是 `NAME_LEN` 那一块（尾部补零）。
    let class = class_block("ns16550a");
    assert_eq!(Name::from_bytes(class), Ok(name("ns16550a")));
    // **太长的类名是截断**（不是拒绝）：照实记——我原来以为它答 `Err(TooLong)`，实测是**截到
    // `NAME_LEN - 1` 再补零**。这条口径有一个值得知道的下场：两个只有尾巴不同的长类名会**撞成
    // 同一格**（下面这一句就是它）。今天没有这么长的类名，故只记口径、不改结构。
    let too_long = "this-name-is-way-too-long-for-one-block";
    assert!(
        Name::new(too_long).is_err(),
        "原串本身就太长（`NAME_LEN` 那一格）"
    );
    let long = class_block(too_long);
    let same_prefix = class_block("this-name-is-way-too-long-for-one-blocc");
    assert_eq!(
        long, same_prefix,
        "前 `NAME_LEN - 1` 字节相同的两个长类名撞成同一格"
    );
    let truncated = Name::from_bytes(long).expect("截断之后是个合法名字");
    assert_eq!(
        truncated.as_str().len(),
        env::NAME_LEN - 1,
        "截到 `NAME_LEN - 1`（留一格给结尾那个零）"
    );
    assert_eq!(truncated.as_str(), &too_long[..env::NAME_LEN - 1]);

    let need = Need::class(class, Kind::Pole, Access::FETCH, Policy::NONE);
    assert_eq!(need.class_name(), Some(name("ns16550a")));
    let settled = need.settle(|name| (name == "ns16550a").then(|| Key::region(0x1000_0000)));
    assert_eq!(
        settled.and_then(|w| w.key()),
        Some(Key::region(0x1000_0000))
    );
    // **这台机器上没有这一类** ⇒ `None`（本层的失败，不是引导域的答话）。
    let need = Need::class(class, Kind::Pole, Access::FETCH, Policy::NONE);
    assert!(need.settle(|_| None).is_none(), "这台机器上没有这一类");

    // 坐标已经知道的那一格：**根本不去查表**（传一个会炸的闭包）。
    let known = Need::known(Key::region(0x2000), Kind::Nole, Access::STORE, Policy::NONE);
    let settled = known.settle(|_| unreachable!("已知坐标不该查表"));
    assert_eq!(settled.and_then(|w| w.key()), Some(Key::region(0x2000)));
}

// ── 那张长度表那一条用例**删了**（用户裁定"常量交给编译器"）────────────────
//
// `the_frame_lengths_and_caps_are_what_the_wire_says` 原先在这里。四条断言里：
// `WANT_LEN == 32` 在 `crates/protocol/src/driver/supply/call.rs` 里**早就是**
// `const _: () = assert!(…)`；另外三条（`WANT_MAX` / `ORDER_CAP` / `REPLY_CAP` 各自等于自己
// 的定义式）是**同义反复** ⇒ 一条都不必再占用例。
#[test]
fn the_supply_failure_table_is_lossy_on_purpose_and_keeps_the_two_nones_apart() {
    use crate::call::{BAD, DENIED, FULL, OK, UNKNOWN, code_to_fail, fail_to_code};

    assert_eq!(fail_to_code(None), OK, "没失败 ⇒ OK");
    assert_eq!(fail_to_code(Some(Fail::Unknown)), UNKNOWN);
    assert_eq!(fail_to_code(Some(Fail::Denied)), DENIED);
    assert_eq!(fail_to_code(Some(Fail::Full)), FULL);
    // **这一张不是双射**（`Local` 与 `Bad` 同归 `BAD`）⇒ 宏不给反向，反向由人写：
    // 解回来的时候这两格都落回 `Bad`（"尽力而为"那一句）。
    assert_eq!(fail_to_code(Some(Fail::Local)), BAD);
    assert_eq!(fail_to_code(Some(Fail::Bad)), BAD);
    assert_eq!(code_to_fail(OK), None, "OK 不是失败");
    assert_eq!(
        code_to_fail(BAD),
        Some(Fail::Bad),
        "两个 BAD 的来源在这里只有一个名字"
    );
    assert_eq!(code_to_fail(UNKNOWN), Some(Fail::Unknown));
    assert_eq!(code_to_fail(DENIED), Some(Fail::Denied));
    assert_eq!(code_to_fail(FULL), Some(Fail::Full));
    assert_eq!(code_to_fail(200), None, "表外的码");
    assert_ne!(BAD, OK, "两者不同码才分得开");
}

#[test]
fn the_two_views_do_not_accept_the_other_family() {
    // **"一位不多"**：`Access` 只收读写族那两位、`Policy` 只收传递族那两位——混族的值不可表达。
    // 照实记：这两个类型原先住 `runtime::core::port`，搬进 `env` 时**逐字照搬**（判据才第一次
    // 落在它们身上）。
    assert_eq!(
        Access::from_bits(Access::FETCH.bits().bits()),
        Some(Access::FETCH)
    );
    assert_eq!(
        Access::from_bits(Access::FETCH_STORE.bits().bits()),
        Some(Access::FETCH_STORE)
    );
    assert_eq!(
        Policy::from_bits(Policy::VEST.bits().bits()),
        Some(Policy::VEST)
    );

    assert_eq!(
        Access::from_bits(Policy::VEST.bits().bits()),
        None,
        "传递族进不了读写那一格"
    );
    assert_eq!(
        Policy::from_bits(Access::STORE.bits().bits()),
        None,
        "读写族进不了传递那一格"
    );
    assert_eq!(
        Access::from_bits(Access::FETCH.bits().bits() | Policy::VEST.bits().bits()),
        None,
        "混族"
    );
    assert_eq!(Policy::from_bits(0xFFFF), None, "未知位");

    // 空集**是合法的值**（两族都空）——"就地拒"是 `ship` 那一手的裁决，不是类型的。
    assert_eq!(Access::from_bits(0), Some(Access::NONE));
    assert_eq!(Policy::from_bits(0), Some(Policy::NONE));

    // 两族各自成对：`Access::FETCH | Access::STORE` 是那一族的并，`Policy` 也有自己的并
    // （两族**不能**互相 `|`——那不是"混族不可表达"，那是类型层面就不给）。
    assert_eq!(Access::FETCH | Access::STORE, Access::FETCH_STORE);
}
