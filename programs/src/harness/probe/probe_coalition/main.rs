#![no_std]
#![no_main]

//! Identity publication acceptance: all seventeen entries exist with their discovery policy.
//! Each entry must carry its action mark and belong to the same Identity authority.
//!
//! # 判据为什么必须 **panic**
//! 整机那一格判的是"有没有 `EXIT_PANIC`"（`kernel/src/work/room/conductor.rs` 的判据只此一处）：
//! 返回一个非零的 `Report` **不算红**。故每一步失败都当场塌。

extern crate programs;

use env::Wait;
use programs::Report;
use programs::harness::probe;

use ::resource::raw::reserve;
use env::unit;
use ipc::session::Session;
use programs::debug;
use system_api::operator::Grant;
use system_api::operator::path::Path;
use system_client::operator;
use system_client::operator::Face;

const MS: usize = 1000;

/// 数一族到齐的**总窗口**（毫秒）：那几位由各自那一域自己落（本台可能比它先起），
/// 故有界——等的是**事件**（`Watch::next`），到点由 `assert_eq!` 落地。
const FACES_MS: usize = 3_000;

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）。
/// **量出来的那两枚数**落在这里（release 也看得见）：`debug!` 在 release 是空操作，
/// 而这一句两边都打——所以"数到几枚"与"该有几枚"（`assert_eq!`）都得看得到。
const OK_NOTE: &str = "probe-coalition: identity=17, all action entries home";

#[programs::entry]
fn main() -> Report<'static> {
    // 一、**先装路**（次序是硬的：这一台只在别的域里说话，故先要树那条会话）。
    let Ok(session) = Session::open(unit::sire(), operator::BERTH, Wait::AtMost(MS)) else {
        panic!("probe-coalition: no tree link");
    };
    let tree = Face::from(&session);

    assert_eq!(system_api::identity::Grant::ALL.len(), 17);
    step(
        &tree,
        "identity",
        system_api::operator::Path::new(system_api::identity::DIR),
        system_api::identity::Grant::ALL.len(),
    );
    let authority = system_client::identity::authority()
        .expect("probe-coalition: no Control-issued identity authority");
    for grant in system_api::identity::Grant::ALL {
        let entry = if grant.mount() == system_api::identity::Mount::Installer {
            let road = system_api::operator::Path::new(system_api::identity::DIR)
                .try_join(grant.name())
                .expect("probe-coalition: bad face name");
            assert_eq!(
                tree.tile(&road, Wait::AtMost(MS))
                    .unwrap()
                    .token(Wait::AtMost(MS)),
                Err(system_api::operator::Fail::Denied),
                "installer discovery must deny an ordinary principal"
            );
            let entry = ipc::session::establish::find(authority, grant.mark())
                .unwrap_or_else(|_| panic!("probe-coalition: installer face missing or ambiguous"));
            assert_eq!(reserve(entry).unwrap().0, unit::sire());
            entry
        } else {
            fetch(
                &tree,
                system_api::operator::Path::new(system_api::identity::DIR),
                grant.name(),
                "identity",
            )
        };
        let (_, owner, mark) =
            reserve(entry).expect("probe-coalition: identity entry cannot be reserved");
        assert_eq!(mark, grant.mark(), "identity action mark mismatch");
        assert_eq!(owner, authority, "identity action has foreign owner");
    }

    return Report::note(env::EXIT_OK, OK_NOTE);
}

/// 数一族：那一块窗格底下到齐没有（该有几枚由调用方那一族的 `Grant::ALL` 说）。
fn step(tree: &Face, family: &str, dir: &Path, want: usize) {
    // **先订**（序是契约）：那一族此后每落一格都往本端这一页记一条，`Watch::of` 返回就是
    // 那个序点——故订阅排在"数一次"之前，已经到齐的族则由量具第一问当场返回。
    // 订要持柄：`watch` 是 `Grant::Watch` 那一维上的一枚（`Face::rein` 借出来）。
    let rein = tree.rein(Grant::Watch);
    let mut watch = match rein.watch(dir, Wait::AtMost(MS)) {
        Ok(watch) => watch,
        Err(fail) => panic!("probe-coalition: {family} 那一族订不成：{fail:?}"),
    };
    let pane = tree
        .pane(dir, Wait::AtMost(MS))
        .unwrap_or_else(|fail| panic!("probe-coalition: {family} 那一格不是窗格：{fail:?}"));
    let seen = probe::count::count_under(&pane, want, &mut watch, FACES_MS);
    debug!("probe-coalition: {family} faces={seen} want={want}");
    assert_eq!(
        seen, want,
        "{family} 底下不对齐（Grant::ALL 有 {want} 枚，数到的只有 {seen} 格）"
    );
}

/// 取回一族某一面的那一枚门牌（`tile` 译号 → `token` 把门闩授进本表）。
fn fetch(tree: &Face, dir: &Path, face: &str, family: &str) -> env::PieToken {
    // 面名由各自那一族的 `Grant` 给（单段、不含 `/`），故这一段拼不出来是**类型写错**，
    // 不是运行期的事——照仓里那几台的排法用 `expect`。
    let road = dir.try_join(face).expect("probe-coalition: bad face name");
    let got = tree
        .tile(&road, Wait::AtMost(MS))
        .and_then(|tile| tile.token(Wait::AtMost(MS)));
    let Ok(entry) = got else {
        panic!("probe-coalition: {family}/{face} 那一枚取不回来（{got:?}）");
    };
    debug!("probe-coalition: {family}/{face} entry={}", entry.get());
    entry
}
