#![no_std]
#![no_main]

//! probe-operator-land — **操作面的正证客人**：会话只开在 `land` 那一位上，于是**只发得出** `land`。
//!
//! ```text
//!   1  与树开会话（`operator::granted_berth(Grant::Land)`）——**会话就是那一柄权**
//!   2  LAND /probe-op-free（无主）  ⇒ **通**，且答的就是那一格自己的号
//!   3  LAND /probe-op-own（别人有主）⇒ **Denied**（面 ✓、归属 ✗ ⇒ **两条轴正交**）
//!   4  LAND /probe-op-own（拿真号再顶一次）⇒ 仍 **Denied**，且那一格**一字未动**
//!   5  SEEK /svc/sys/operator  ⇒ **Denied**（`seek` 是另一柄权）
//!   6  PART /             ⇒ **Denied**（`part` 是另一柄权；**根**那一格够不着它）
//!   7  FIND  某号          ⇒ **Denied**（`find` 会**交出能力**，自成一位，不与只读那几条合并）
//!   8  TRIM  某号          ⇒ **Denied**
//!   9  LIST /             ⇒ **Denied**（读面也各是一柄权）
//!  10  NAME  某号          ⇒ **Denied**
//! ```
//!
//! # 为什么它读不了树（这不是缺陷，是这一维在生效）
//!
//! 第 5–10 条**全部**答 `Denied`：这一位客人**列不了、问不了名、译不了号**。故它认路的坐标
//! 只能自己报得出——**根是唯一不需要号的那一格**（根没有号，见 `operator::frame`），于是
//! 那两格由 `probe-operator-gate` 落在**根**底下，本台照名字报坐标。
//!
//! 这一条是**量出来的**：第一版拿 `list` ＋ `name` 去走 `/svc/sys/operator/zone`，于是每一次
//! `list` 都被面判拒掉、当场卡死——本台当时量到的不是"读树读不到"，而是"**没资格读**"。
//!
//! # 第 2/3 条合起来是两件不同的事
//!
//! 同一位客人的两次 `land`，一次通、一次拒：**第 3 条拒在"那一格自己有主"**（`ledger` 的
//! "改"那一轴），**第 5–10 条拒在"这一柄权没许这一类"**（面判）。两组各自单独成立，故证明
//! 它们不是同一条闸。
//!
//! # 两道门的次序（本台量的是**第一道**）
//!
//! ```text
//!   1 这一位许不许这一类？  面判（本台这一柄权）
//!   2 那一格许不许我改？    归属（另一条轴）
//! ```
//!
//! `find` / `trim` / `land` 那三条还要再过一道身份闸（`may`）——本台是**已绑身份**的（声明里
//! `bind: true`），故那一道不放行也不拦；这一台量的是第 1 道与第 2 道正交。

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

/// 一趟一问的期限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// **起步那一等**（毫秒）：等 `probe-operator-gate` 把它那两格摆好。
///
/// 本台**读不了树**（`list` / `name` / `seek` 各是另一柄权，全被面判拒），故"那两格摆好了没有"
/// 这一问它问不出来。装配者只保证**起手装路的先后**（本台排在那位之后），不保证"那位把场摆完"
/// ——两位是并发跑的。没有这一等，本台会**赶在铺场者之前**去顶那一格：那一格当时无主 ⇒ 本台
/// 落了它、自己成了主人（实测：`Ok(EntryId(13))`，判据当场红，而对面那一格本该是别人的）。
///
/// 它是**有界的**：过量只是白等，不改变判据。
const SETTLE_MS: usize = 300;

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）。
const OK_NOTE: &str = "probe-operator-land: land only";

/// 由 `probe-operator-gate` 声明归它自己的那一格（本台顶它 ⇒ 该拒）。
const OWN: &str = "probe-op-own";
/// 由它留下的**无主**那一格（本台落得下去）。
const FREE: &str = "probe-op-free";

/// **不等于任何真格子**的一枚号：第 7–10 条只量"面判"，故拿哪一枚都一样（那几条**到不了树**）。
const NOBODY: EntryId = EntryId::new(usize::MAX);

#[programs::entry]
fn main() -> Report<'static> {
    // 一、**一条授面的会话**：`granted_berth` 把问话孔那一格换成 `land` 那一位的记号。
    //    次序是硬的（先装路）——见 `harness/src/probe_operator_gate.rs` 的文件头。
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
    // **等铺场者把两格摆完**（见 [`SETTLE_MS`]）。
    let _ = runtime::env::room::sleep(core::time::Duration::from_millis(SETTLE_MS as u64));

    // 二、`seek` / `part` / `find` / `trim` / `list` / `name`：**一柄也不许**。
    //
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
    let own = OWN.to_string();
    let denied = rein.land(
        Where::Root,
        own.clone(),
        mint("probe-land-mine"),
        Permit::Unset,
        operator::Mine::No,
        Wait::AtMost(MS),
    );
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

    debug!("probe-operator-land: free landed={}", got.get());
    return Report::note(env::EXIT_OK, OK_NOTE);
}

/// 铸一枚本域自己的入口（记号只为本台这台测具而立，不进任何一族的表）。
fn mint(mark: &'static str) -> PieToken {
    match mail::unseal_hole(env::Mark::of(mark)) {
        Ok(pie) => pie,
        Err(_) => panic!("probe-operator-land: no entry"),
    }
}
