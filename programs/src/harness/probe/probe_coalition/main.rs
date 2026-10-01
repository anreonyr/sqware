#![no_std]
#![no_main]

//! probe-coalition — **那两族"没有会话"的服务**（名册 / 盟册）的格数判据。
//!
//! 判据两条：
//! 1. `/svc/sys/principal` 与 `/svc/sys/coalition` 底下**各该有 `Grant::ALL.len()` 枚**
//!    （一枚 Grant = 一枚门牌 = 一格）——与 `probe-operator-gate` 数 `/svc/sys/operator`、
//!    `probe-control` 数 `/svc/sys/control` **同一把尺子**：四族各归各家。
//! 2. 那四枚门牌（`ask` / `set` 各两枚）**都取得回来**（`find` 那一问会把它授进本表）。
//!
//! # 为什么这一台该存在
//! 那两族落格那一侧是**静默**的：`let _ = bridge::land(…)`（`principal/serve`、`coalition/serve`）
//! 把 `Landed { land, find }` 丢掉——**少落一格照样起**，只有树上少一格。故"到齐没有"这一问
//! 必须由**别的域**的人来问，这正是本台。
//!
//! # 判据为什么必须 **panic**
//! 整机那一格判的是"有没有 `EXIT_PANIC`"（`kernel/src/work/room/conductor.rs` 的判据只此一处）：
//! 返回一个非零的 `Report` **不算红**。故每一步失败都当场塌。

extern crate programs;

use env::Wait;
use programs::Report;
use programs::harness::probe;

use protocol::common::path::Path;
use protocol::communication::session::Session;
use protocol::debug;
use protocol::service::coalition as ccall;
use protocol::service::operator::client as operator;
use protocol::service::operator::client::Face as TreeFace;
use protocol::service::principal as pcall;
use runtime::env::unit as utask;

const MS: usize = 1000;

/// 数一族到齐的额度与节拍（毫秒）：那几位由各自那一域自己落（本台可能比它先起），
/// 故有界重试，到点由 `assert_eq!` 落地。
const FACES_MS: usize = 3_000;

/// 每一次重试之间睡多久（毫秒）
const TICK_MS: usize = 20;

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）。
/// **量出来的那两枚数**落在这里（release 也看得见）：`debug!` 在 release 是空操作，
/// 而这一句两边都打——所以"数到几枚"与"该有几枚"（`assert_eq!`）都得看得到。
const OK_NOTE: &str = "probe-coalition: principal=2 coalition=2, four plates home";

#[programs::entry]
fn main() -> Report<'static> {
    // 一、**先装路**（次序是硬的：这一台只在别的域里说话，故先要树那条会话）。
    let Ok(session) = Session::open(utask::sire(), operator::BERTH, Wait::AtMost(MS)) else {
        panic!("probe-coalition: no tree link");
    };
    let tree = TreeFace::from(&session);

    // 二、**两族各数一遍**：`road` 是那一族的 `DIR`，`want` 是那一族 `Grant::ALL.len()`
    //    （两族都是 2：`ask` 问面 ＋ `set` 定面）。数不满由 `step` 里那句 `assert_eq!` 当场红，
    //    而"该有几枚"那个数由各自的 `Grant::ALL` 说——本台不另记一份。
    step(&tree, "principal", pcall::DIR, pcall::Grant::ALL.len());
    step(&tree, "coalition", ccall::DIR, ccall::Grant::ALL.len());

    // 三、**四枚门牌都取得回来**：不是只数格——`find` 那一问会把那一枚授进本表，
    //    这才是"那一格后面真有东西"（对照 `probe-control` 量过的那一件反面：挂得出来、取不回来）。
    for grant in pcall::Grant::ALL {
        fetch(&tree, pcall::DIR, grant.name(), "principal");
    }
    for grant in ccall::Grant::ALL {
        fetch(&tree, ccall::DIR, grant.name(), "coalition");
    }

    return Report::note(env::EXIT_OK, OK_NOTE);
}

/// 数一族：那一块窗格底下到齐没有（该有几枚由调用方那一族的 `Grant::ALL` 说）。
fn step(tree: &TreeFace, family: &str, dir: &Path, want: usize) {
    let pane = tree
        .pane(dir, Wait::AtMost(MS))
        .unwrap_or_else(|fail| panic!("probe-coalition: {family} 那一格不是窗格：{fail:?}"));
    let seen = probe::count::count_under(&pane, want, FACES_MS, TICK_MS);
    debug!("probe-coalition: {family} faces={seen} want={want}");
    assert_eq!(
        seen, want,
        "{family} 底下不对齐（Grant::ALL 有 {want} 枚，数到的只有 {seen} 格）"
    );
}

/// 取回一族某一面的那一枚门牌（`tile` 译号 → `token` 把门闩授进本表）。
fn fetch(tree: &TreeFace, dir: &Path, face: &str, family: &str) {
    // 面名由各自那一族的 `Grant` 给（`name_max` 之内、不含 `/`），故这一段拼不出来是**类型写错**，
    // 不是运行期的事——照仓里那几台的排法用 `expect`。
    let road = dir
        .try_join(face)
        .expect("probe-coalition: bad face name");
    let got = tree
        .tile(&road, Wait::AtMost(MS))
        .and_then(|tile| tile.token(Wait::AtMost(MS)));
    let Ok(entry) = got else {
        panic!("probe-coalition: {family}/{face} 那一枚取不回来（{got:?}）");
    };
    debug!("probe-coalition: {family}/{face} entry={}", entry.get());
}
