//! 编排那一层的门（**宿主台**）—— **账 / 判定 / 配给**三份核心，在宿主上真跑一遍。
//!
//! # 这一台钉的是什么
//!
//! `crates/protocol/src/system/{desk,core,grant}.rs` 三份都是纯的（只认 `env`，`core.rs` 另认
//! 同层 `desk.rs` 的那几个类型），故这一台**无桩**。规格写在靶子里（照 `protocol-case` 的 `line` 靶的
//! 做法），三份源码**一个字不动**：
//!
//! ```text
//!   账    register 撞名 ⇒ Unknown；find / rows 只认**有名字的**行；
//!         attach 没登记过 ⇒ Unknown，且"身子挂上"不等于"起来了"（state 照旧 NeverStarted）；
//!         detach 摘身子**留行**（"起过、现在死了"说得出）；set_state 找不到名字 ⇒ 什么都不动；
//!         表满 ⇒ NoRoom（CAP 是常量，与清单上限同值）
//!   判定  admit_start：不在表里 ⇒ Unknown；Dead / NeverStarted ⇒ Ok（**重发那一格**）；
//!         其余 ⇒ NotReady
//!         probe_ready：按**这一行自己声明的**说法解读（不宣布的那种"放行即起来"）
//!         probe_watch：表里还有没有那对坐标（**不是生死**，见那份源码的照实记）
//!   配给  each：第 i 条交给第 i 格；不足一条的尾巴不回调
//! ```

extern crate alloc;

mod system;

use env::{Name, TaskId, TeamId};
use system::core::{Fail, Ready, Watch, admit_start, probe_ready, probe_watch};
use system::desk::{Announce, Service, Slot, State, Table};

fn name(text: &str) -> Name {
    Name::new(text).expect("名字合法")
}

/// 一张登记了两行的表：一行"要通道的"、一行"不宣布的"。
fn table() -> Table {
    let mut t = Table::new();
    t.register(name("uart"), Announce::Channel)
        .expect("登记得下");
    t.register(name("echo"), Announce::None).expect("登记得下");
    t
}

fn live(task: usize) -> (TeamId, TaskId) {
    (TeamId::new(3), TaskId::new(task))
}

// ── 账 ──────────────────────────────────────────────────────

#[test]
fn register_refuses_a_second_row_with_the_same_name() {
    // 名字是**表内的唯一坐标**：同名再登记是调用方写错了（`Unknown`），不是"更新"。
    let mut t = Table::new();
    assert_eq!(t.register(name("uart"), Announce::Channel), Ok(()));
    assert_eq!(t.register(name("uart"), Announce::None), Err(Fail::Unknown));
    assert_eq!(t.rows().count(), 1, "那一行没有被改写");
    assert_eq!(
        t.find(name("uart")).map(|s| s.announce),
        Some(Announce::Channel),
        "先来的那一行照旧"
    );
}

#[test]
fn find_and_rows_only_see_named_rows() {
    // 表是**定长数组**：没名字的那些格是"占着位但不算一行"——`find` / `rows` 都不该看见它们。
    let t = table();
    assert_eq!(t.find(name("never-registered")), None);
    assert_eq!(t.rows().count(), 2);
    assert!(
        t.rows()
            .all(|s| s.name == name("uart") || s.name == name("echo"))
    );
}

#[test]
fn attaching_a_body_needs_a_registered_row_and_is_not_yet_ready() {
    // `attach` 挂的是**身子**（域 + 代表线程）；没登记过 ⇒ `Unknown`。
    // 而挂上身子**不等于起来了**：`state` 照旧 `NeverStarted`、通道照旧没有。
    let mut t = table();
    assert_eq!(
        t.attach(name("nobody"), TeamId::new(3), TaskId::new(7)),
        Err(Fail::Unknown)
    );

    let (team, task) = live(7);
    assert_eq!(t.attach(name("uart"), team, task), Ok(()));
    let s: &Service = t.find(name("uart")).expect("在");
    assert_eq!(s.slot, Slot::Live { team, task });
    assert_eq!(s.state, State::NeverStarted, "挂上身子 ≠ 起来了");
    assert_eq!(s.root, None, "通道要等它交回来");
}

#[test]
fn detaching_keeps_the_row_so_it_can_still_say_it_once_ran() {
    // **摘身子留行**（`Slot::None`、状态与名字照旧）：表要能说出"起过、现在死了"。
    let mut t = table();
    let (team, task) = live(7);
    t.attach(name("uart"), team, task).unwrap();
    t.set_state(name("uart"), State::Ready);
    t.detach(name("uart"));

    let s = t.find(name("uart")).expect("行还在");
    assert_eq!(s.slot, Slot::None, "身子摘了");
    assert_eq!(s.root, None);
    assert_eq!(
        s.state,
        State::Ready,
        "**状态照旧**——它就是「起过」那半句话"
    );
    assert!(s.name == name("uart"));
    // 摘一个没登记过的名字：什么都不发生（不 panic）。
    t.detach(name("nobody"));
    assert_eq!(t.rows().count(), 2);
}

#[test]
fn set_state_with_a_wrong_name_touches_nothing() {
    let mut t = table();
    t.attach(name("uart"), TeamId::new(3), TaskId::new(7))
        .unwrap();
    t.set_state(name("nobody"), State::Ready);
    assert_eq!(
        t.find(name("uart")).map(|s| s.state),
        Some(State::NeverStarted)
    );
}

#[test]
fn the_table_has_a_bottom() {
    // **条数是策略、容器要有界**：装满了答 `NoRoom`（不是 panic、也不是悄悄覆盖别人）。
    let mut t = Table::new();
    let mut made = 0;
    loop {
        // 名字各不相同（`s{i}`）。
        let text = alloc::format!("s{made}");
        let Ok(one) = Name::new(&text) else {
            panic!("名字合法")
        };
        match t.register(one, Announce::None) {
            Ok(()) => made += 1,
            Err(Fail::NoRoom) => break,
            Err(other) => panic!("不该是别的错：{other:?}"),
        }
    }
    assert_eq!(made, Table::CAP, "登记得下的正好是 CAP 行");
    assert_eq!(t.rows().count(), Table::CAP);
}

// ── 判定 ────────────────────────────────────────────────────

#[test]
fn admit_start_says_unknown_outside_the_table_and_not_ready_while_running() {
    let mut t = table();
    assert_eq!(admit_start(&t, name("nobody")), Err(Fail::Unknown));

    t.attach(name("uart"), TeamId::new(3), TaskId::new(7))
        .unwrap();
    assert_eq!(admit_start(&t, name("uart")), Ok(()), "没起过 ⇒ 准起");

    for state in [State::Starting, State::Ready, State::Stopping] {
        t.set_state(name("uart"), state);
        assert_eq!(
            admit_start(&t, name("uart")),
            Err(Fail::NotReady),
            "{state:?} 时不该再准起"
        );
    }
    // **重发那一格**：`Dead` 与 `NeverStarted` 同档 ⇒ 同一行上再起是允许的。
    t.set_state(name("uart"), State::Dead);
    assert_eq!(admit_start(&t, name("uart")), Ok(()));
}

#[test]
fn probe_ready_reads_the_way_that_row_said_it_would_announce() {
    let mut t = table();
    assert_eq!(
        probe_ready(&t, name("nobody")),
        Ready::Gone,
        "不在表里 ⇒ 没了"
    );

    // 不宣布的那一种：**放行即起来**（身子在 ⇒ Up）。
    t.attach(name("echo"), TeamId::new(3), TaskId::new(8))
        .unwrap();
    t.set_state(name("echo"), State::Starting);
    assert_eq!(probe_ready(&t, name("echo")), Ready::Up);

    // 要通道的那一种：身子在、还没宣布 ⇒ 继续等。
    t.attach(name("uart"), TeamId::new(3), TaskId::new(7))
        .unwrap();
    t.set_state(name("uart"), State::Starting);
    assert_eq!(probe_ready(&t, name("uart")), Ready::Pending);

    // 宣布过了 ⇒ Up；而身子不在（`Slot::None`）⇒ 它没起来。
    t.set_state(name("uart"), State::Ready);
    assert_eq!(probe_ready(&t, name("uart")), Ready::Up);
    t.detach(name("uart"));
    t.set_state(name("uart"), State::Starting);
    assert_eq!(probe_ready(&t, name("uart")), Ready::Gone);

    // 收尾中 / 起过又死了 ⇒ Gone。
    t.set_state(name("uart"), State::Stopping);
    assert_eq!(probe_ready(&t, name("uart")), Ready::Gone);
    t.set_state(name("uart"), State::Dead);
    assert_eq!(probe_ready(&t, name("uart")), Ready::Gone);
    t.set_state(name("uart"), State::NeverStarted);
    assert_eq!(probe_ready(&t, name("uart")), Ready::Gone);
}

#[test]
fn probe_watch_is_about_the_coordinates_not_about_life_and_death() {
    // **照实记**：这一对名字（`Alive`/`Gone`）名不副实——死亡记账**不清坐标**，故一位已经
    // 收尾的 Service 在这里仍答 `Alive`。生死要看 `State`。这一格把那条口径钉住。
    let mut t = table();
    assert_eq!(probe_watch(&t, name("nobody")), Watch::Gone);
    t.attach(name("uart"), TeamId::new(3), TaskId::new(7))
        .unwrap();
    t.set_state(name("uart"), State::Ready);
    assert_eq!(probe_watch(&t, name("uart")), Watch::Alive);
    // 收尾完了（状态是死的），坐标还在 ⇒ 仍答 Alive——**这不是生死**。
    t.set_state(name("uart"), State::Dead);
    assert_eq!(probe_watch(&t, name("uart")), Watch::Alive);
    // 摘了身子（坐标没了）⇒ Gone。
    t.detach(name("uart"));
    assert_eq!(probe_watch(&t, name("uart")), Watch::Gone);
}

// ── 配给 ────────────────────────────────────────────────────

#[test]
fn grant_each_hands_record_i_to_cell_i() {
    // **第 i 条就是单子第 i 条的答**（同序同长）：收方那张表按位次归位，本模块不解释坐标。
    use env::{Key, PAIR_LEN, Pair};

    let a = Pair::bytes(Key::region(0x1000_0000), 11);
    let b = Pair::bytes(Key::irq(), 12);
    let mut records = alloc::vec::Vec::new();
    records.extend_from_slice(&a);
    records.extend_from_slice(&b);
    records.extend_from_slice(&[0u8; PAIR_LEN - 1]); // 不足一条的尾巴：不许被解出来

    let mut got = alloc::vec::Vec::new();
    system::grant::each(&records, |i, pair| {
        got.push((i, pair.key(), pair.token().get()))
    });

    assert_eq!(got.len(), 2, "尾巴不该回调");
    assert_eq!(got[0].0, 0);
    assert_eq!(got[0].1, Key::region(0x1000_0000).into());
    assert_eq!(got[0].2, 11);
    assert_eq!(got[1].0, 1);
    assert_eq!(got[1].2, 12);
}
