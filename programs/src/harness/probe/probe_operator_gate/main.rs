#![no_std]
#![no_main]

//! Verify all Operator grants and reject raw mutation through the generic session.

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;
use programs::harness::probe;

use env::pie;
use env::unit;
use ipc::session::{Session, establish};
use system_api::operator::EntryId;
use system_api::operator::Fail;
use system_api::operator::Grant;
use system_api::operator::Permit;
use system_api::operator::path::Path;
use system_client::operator;
use system_client::operator::Face;
use system_client::operator::Mine;
use system_client::operator::Pane;
use system_client::operator::Watch;

const MS: usize = 1000;

/// **等那一段目录长出来 / 那几格到齐 / 下一位走完**的额度（毫秒）：三处（`walk`、`count_under`
/// 与末尾那一条 `DONE_ROAD`）各拿它当**总窗口**——等的是**事件**（`watch.next`），
/// 不再是"睡一拍再看"，故节拍那一格没有了。
/// **（20 s → 3 s）**：本台是**铺场者**，而它"走不完"的代价不是红——停机扳机一来就把它
/// **扑杀**（`ousted=true`、一行不打），那条读数于是**既不绿也不红**（第三种结局）。旧版给的是
/// 20 s，而这样的额度本台有**两处**（`walk` 与 `count_under`），走满就是几十秒的窗口。
/// 收紧到 3 s 之后：健康那一档（实测走完全程只要 1~3 s）毫发无伤，
/// 而"数不到"那一档**当场红**（到点返回、由调用方那句 `assert` 落地）。
/// 这个数不是猜的：七行 `system: grant mounted at /svc/sys/operator/{…}` 在 **t<500 ms** 就打完
/// （同一份镜像、直接起 QEMU 量过），故一个控制面会话看得见它们的时间以毫秒计
const WAIT_MS: usize = 3_000;

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）
const OK_NOTE: &str = "probe-operator-gate: eight grants mounted";

#[programs::entry]
fn main() -> Report<'static> {
    // 一、**先装路**（次序是硬的，见文件头）。
    let Ok(session) = Session::open(unit::sire(), operator::BERTH, Wait::AtMost(MS)) else {
        panic!("probe-operator-gate: no tree link");
    };
    let tree = Face::of(session);

    // 一·二、**先订**（序是契约）：`Watch::of` 返回就是那个序点——本台此后每一处都是
    //       "先问一次、不满才等事件"，故订阅必须排在"等那一块长出来"之前。
    //       `spot` 那一处不在这条路上：它等的是"这一趟**走到了**"（单槽孔的信），不是树变了。
    let mut watch = match tree.watch(&system_api::operator::DIR, Wait::AtMost(MS)) {
        Ok(watch) => watch,
        Err(fail) => panic!("probe-operator-gate: subscribe /svc/sys/operator failed: {fail:?}"),
    };
    let Some(operator_id) = walk(&tree, &system_api::operator::DIR, &mut watch) else {
        panic!("probe-operator-gate: /svc/sys/operator is not a pane");
    };
    let operator_pane = Pane::of(&tree, operator_id);

    // 三、那几位到齐：**一问 ＋ 等事件**（各位名字的读数归树自己那几行 `grant mounted at`，
    //    见 `probe::count::count_under`）。**该有几枚由 `Grant::ALL` 说**——加一位就跟着动。
    let seen = count_under(&operator_pane, &mut watch);
    assert_eq!(
        seen,
        Grant::ALL.len(),
        "/svc/sys/operator 底下不对齐（Grant::ALL 有 {} 枚，数到的只有 {seen} 格）",
        Grant::ALL.len()
    );

    // 四、`/svc/sys/operator/land`：**整条路**译号 → **取回那一枚入口**（`find` 把它授进本表）。
    //    Face::tile 收的是**从根写起**的那条路（见 `client.rs` 的 Pane::tile 那一节）
    let road = system_api::operator::DIR
        .try_join("land")
        .expect("probe-operator-gate: bad name");
    let Ok(tile) = tree.tile(&road, Wait::AtMost(MS)) else {
        panic!("probe-operator-gate: /svc/sys/operator/land is not on the tree");
    };
    let land_id = tile.id();
    let Ok(cap) = tile.token(Wait::AtMost(MS)) else {
        panic!("probe-operator-gate: find /svc/sys/operator/land was refused");
    };
    // 那一枚是**入口**（孔），不是随便一枚号：它答得出开者（铸它的就是编排域主线程）。
    assert!(
        establish::opened_by(cap).is_some(),
        "取回来的那一枚不是一枚入口（问不出开者）"
    );

    let root = tree.root();
    assert!(matches!(
        root.open("idt".into(), Wait::AtMost(MS)),
        Err(Fail::Denied)
    ));
    let source = pie::unseal_hole(env::Mark::of("raw-generic")).unwrap();
    assert!(matches!(
        root.bind(
            "uit".into(),
            source,
            Permit::Public,
            Mine::No,
            Wait::AtMost(MS)
        ),
        Err(Fail::Denied)
    ));
    assert!(matches!(
        root.trim(land_id, Wait::AtMost(MS)),
        Err(Fail::Denied)
    ));
    programs::debug::put("hierarchy: generic Part/Land/Trim denied for bound Task");
    Report::note(env::EXIT_OK, OK_NOTE)
}

/// `/svc/sys/operator` 那一格自己的号——**一问 ＋ 等事件**：那一块由**别的域**立
/// （本台可能比它先起），"长出来了"那件事就是一条 `Landed`。
/// 三手都是 Face 上现成的手：`root().tile(路)` 译号（**只译号**，不取门闩）、
/// `Tile::id()` 答号、`Tile::pane()` 判"是不是一块 Pane"
fn walk(tree: &Face, road: &Path, watch: &mut Watch<'_>) -> Option<EntryId> {
    let root = tree.root();
    loop {
        // **认得出就是认出了**：`pane` 那一问失败 ⇒ 那一格此刻还不是一块窗格 ⇒ 等一条事件。
        if let Ok(tile) = root.tile(road, Wait::AtMost(MS)) {
            let id = tile.id();
            if tile.pane(Wait::AtMost(MS)).is_ok() {
                return Some(id);
            }
        }
        // 期限内没有事件（那一块始终没长出来）⇒ 答 `None`，由调用方那句 `panic!` 落地。
        if watch.next(Wait::AtMost(WAIT_MS)).is_err() {
            return None;
        }
    }
}

/// 数 `/svc/sys/operator` 底下**那几格到齐没有**——正文在 `harness::probe::count`
/// （各台共用：一事一处）。本台只把"该有几枚"与额度交出去：**该有几枚 = `Grant::ALL.len()`**
/// （一枚 `Grant` = 一枚门牌 = 一格）；额度是本台那一格 `WAIT_MS`（**总窗口**，节拍归量具里的
/// "一问的期限"）。
fn count_under(pane: &Pane<'_>, watch: &mut Watch<'_>) -> usize {
    probe::count::count_under(pane, Grant::ALL.len(), watch, WAIT_MS)
}
