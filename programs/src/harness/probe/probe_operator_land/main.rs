#![no_std]
#![no_main]

//! probe-operator-land — 操作面的正证客人：会话只开在 land 那一位上，于是只发得出 land。
//! # 为什么它读不了树（这不是缺陷，是这一维在生效）
//! 第 5–10 条**全部**答 `Denied`：这一位客人**列不了、问不了名、译不了号**。故它认路的坐标
//! 只能自己报得出——**根是唯一不需要号的那一格**（根没有号，见 operator::frame），于是
//! `probe-op-own` 由 `probe-operator-gate` 声明归属、`probe-op-free` 无主，两格都落在**根**底下，
//! 本台照名字报坐标。（本台先落、对面后换绑也收得住：换绑不动号，见第四步那一节。）
//! 这一条是**量出来的**：拿 `list` ＋ `name` 去走 `/svc/sys/operator/zone`，于是每一次
//! `list` 都被面判拒掉、当场卡死——本台量到的不是"读树读不到"，而是"**没资格读**"。
//! # 第 2/3 条合起来是两件不同的事
//! 同一位客人的两次 `land`，一次通、一次拒：**第 3 条拒在"那一格自己有主"**（`ledger` 的
//! "改"那一轴），**第 5–10 条拒在"这一柄权没许这一类"**（面判）。两组各自单独成立，故证明
//! 它们不是同一条闸。
//! # 两道门的次序（本台量的是**第一道**）
//! `find` / `trim` / `land` 那三条还要再过一道身份闸（`may`）——本台是**已绑身份**的（声明里
//! `bind: true`），故那一道不放行也不拦；这一台量的是第 1 道与第 2 道正交。
//! # 两台之间不靠钟（这一条是量出来的）
//! 从前本台睡一拍（`SETTLE_MS = 300`）再去顶那一格、对面压住那一格（`HOLD_MS = 1200`）保
//! "主人还在场"——**两个都是猜的数**，两头各塌过一次（`Ok(EntryId(13))`、`Ok(EntryId(73))`，
//! 后者就是 80 跑那一档里那条红）。现在两侧都等**事实**：本台按自己那一问的答重试到 `Denied`
//! （见 `WAIT_MS`），对面等本台第六步落的那一格推回来的一条事件才走（见 `DONE`）。

extern crate alloc;
extern crate programs;

use alloc::string::ToString;

use env::Wait;
use programs::Report;

use env::PieToken;
use protocol::communication::session::Session;
use protocol::debug;
use protocol::service::operator::client as operator;
use protocol::service::operator::client::Face as TreeFace;
use protocol::service::operator::{EntryId, Fail, Grant, Permit, Where};
use runtime::env::mail;
use runtime::env::unit as utask;

const MS: usize = 1000;

/// **等那一格被声明有主**的额度（毫秒）。**本台读不了树**（`list` / `name` / `seek` 各是另一柄
/// 权，全被面判拒），故"那一格有主了没有"这件事**只有本台自己那一问说得清**：
///   · `land` 答 `Ok` ⇒ 那一格此刻**无主**（铺场者还没落 / 它的主人已不在场）⇒ 这一趟不算数；
///   · 答 `Denied` ⇒ 那一格**有主且主人在场** ⇒ 判据落定。
/// **它替掉的是从前那一格睡**（`SETTLE_MS = 300`）：睡多久只是个猜的数——短了那一格还没主，
/// 本台自己落下去，判据当场红（实测：`Ok(EntryId(13))`、`Ok(EntryId(73))` 各一次，
/// 都在 80 跑那一档里采到）。额度只留给**失败那条路**（铺场者没了），正常两三趟、毫秒级。
const WAIT_MS: usize = 2_000;

/// 两趟之间歇多久（毫秒）。**不是节拍**：本台每一趟本身就是一问（要一次往返），
/// 这一歇只是不把孔上那一条队列填满。
const RETRY_MS: usize = 10;

/// **走完那一步是一件事**：本台落这一格 ⇒ 铺场者才走得掉（它等这一条事件）。
/// 落在这里而不是"报一句"：本台**没有**给对面送信的路（它读不了树、也不该有第二条会话），
/// 而**落一格**这件事持树者会推给订得起的人——这是本台手里唯一说得响的"我走完了"。
const DONE: &str = "probe-op-done";

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）
const OK_NOTE: &str = "probe-operator-land: land only";

/// 由 `probe-operator-gate` 声明归它自己的那一格（本台顶它 ⇒ 该拒）
const OWN: &str = "probe-op-own";
/// 由它留下的**无主**那一格（本台落得下去）
const FREE: &str = "probe-op-free";

/// **不等于任何真格子**的一枚号：第 7–10 条只量"面判"，故拿哪一枚都一样（那几条**到不了树**）
const NOBODY: EntryId = EntryId::new(usize::MAX);

#[programs::entry]
fn main() -> Report<'static> {
    //    次序是硬的（先装路）——见 `programs/src/harness/probe/probe_operator_gate/main.rs` 的文件头。
    let Ok(session) = Session::open(
        utask::sire(),
        operator::granted_berth(Grant::Land),
        Wait::AtMost(MS),
    ) else {
        panic!("probe-operator-land: no granted session");
    };
    let face = TreeFace::of(session);
    let rein = face.rein(Grant::Land);
    assert!(rein.grant() == Grant::Land, "本台这一柄权不是 land 那一位");

    // 二、`seek` / `part` / `find` / `trim` / `list` / `name`：**一柄也不许**。
    //    这六条**一律到不了树**（面判在第一道就把它挡了），故参数拿哪一枚都不改变结论：
    //    它们量的是"这一位许不许这一类"，不是"那一格在不在"。
    let part = "probe-op-part".to_string();
    // 这一问的**参数是哪条路都不改变结论**（面判在第一道就挡了）——照旧拿本族那一块。
    let sought = rein.seek(&protocol::service::operator::DIR, Wait::AtMost(MS));
    assert!(
        matches!(sought, Err(Fail::Denied)),
        "这一柄权不许 seek，却答了 {sought:?}"
    );
    let parted = rein.part(Where::Root, part, Wait::AtMost(MS));
    assert!(
        matches!(parted, Err(Fail::Denied)),
        "这一柄权不许 part，却答了 {parted:?}"
    );
    let found = rein.find(NOBODY, Wait::AtMost(MS));
    assert!(
        matches!(found, Err(Fail::Denied)),
        "这一柄权不许 find，却答了 {found:?}"
    );
    let trimmed = rein.trim(NOBODY, Wait::AtMost(MS));
    assert!(
        matches!(trimmed, Err(Fail::Denied)),
        "这一柄权不许 trim，却答了 {trimmed:?}"
    );
    let listed = rein.list(Where::Root, Wait::AtMost(MS));
    assert!(
        matches!(listed, Err(Fail::Denied)),
        "这一柄权不许 list，却答了 {listed:?}"
    );
    let named = rein.name(NOBODY, Wait::AtMost(MS));
    assert!(
        matches!(named, Err(Fail::Denied)),
        "这一柄权不许 name，却答了 {named:?}"
    );

    // 三、**无主那一格**：面 ✓ ＋ 归属 ✓ ⇒ 通，且答的就是那一格自己的号。
    let free = FREE.to_string();
    let got = rein.land(
        Where::Root,
        free,
        mint("probe-land-got"),
        Permit::Unset,
        operator::Mine::No,
        Wait::AtMost(MS),
    );
    let got = match got {
        Ok(id) => id,
        Err(fail) => panic!("probe-operator-land: 无主那一格本该落得下去，却答了 {fail:?}"),
    };

    // 四、**别人有主那一格**：面 ✓（`land` 正是这一位）、归属 ✗ ⇒ 拒。
    //    **这一趟等的是事实**（见 WAIT_MS）：本台读不了树，故"那一格有主了没有"只有本台自己
    //    这一问说得清——`Ok` = 此刻无主（本台这一落没改变判据，再来），`Denied` = 有主且在
    //    场，判据落定。对面那一手（`spot`）在已占那一格上是**换绑**，故"谁先落"两种次序都收得住。
    let own = OWN.to_string();
    let mut left = WAIT_MS;
    let mut denied = rein.land(
        Where::Root,
        own.clone(),
        mint("probe-land-mine"),
        Permit::Unset,
        operator::Mine::No,
        Wait::AtMost(MS),
    );
    while denied.is_ok() && left > 0 {
        left = left.saturating_sub(RETRY_MS);
        let _ = runtime::env::room::sleep(core::time::Duration::from_millis(RETRY_MS as u64));
        denied = rein.land(
            Where::Root,
            own.clone(),
            mint("probe-land-mine"),
            Permit::Unset,
            operator::Mine::No,
            Wait::AtMost(MS),
        );
    }
    assert!(
        matches!(denied, Err(Fail::Denied)),
        "顶别人声明归自己的那一格本该被拒，却答了 {denied:?}"
    );

    // 五、**再顶一次**（与第四步同一句话）：仍拒——"拒"不是一次性的。
    let again = rein.land(
        Where::Root,
        own,
        mint("probe-land-again"),
        Permit::Unset,
        operator::Mine::No,
        Wait::AtMost(MS),
    );
    assert!(
        matches!(again, Err(Fail::Denied)),
        "第二次顶有主那一格本该仍被拒，却答了 {again:?}"
    );

    // 六、**说一句"我走完了"**：铺场者那一台此刻正**压着那两格**（`claimable` 判的是"主人还在
    //    不在场"），它等这一条事实才走得掉——本台落一格，持树者把它推给订着那条路的那一位。
    //    **次序是硬的**：这一手排在第四/五步之后，故它一到，"判据已经落定"这件事就同时成立。
    let done = DONE.to_string();
    let Ok(at) = rein.land(
        Where::Root,
        done,
        mint("probe-land-done"),
        Permit::Unset,
        operator::Mine::No,
        Wait::AtMost(MS),
    ) else {
        panic!("probe-operator-land: 走完那一格本该落得下去");
    };

    debug!("probe-operator-land: free landed={} done={}", got.get(), at.get());
    return Report::note(env::EXIT_OK, OK_NOTE);
}

fn mint(mark: &'static str) -> PieToken {
    match mail::unseal_hole(env::Mark::of(mark)) {
        Ok(pie) => pie,
        Err(_) => panic!("probe-operator-land: no entry"),
    }
}
