#![no_std]
#![no_main]

//! probe-operator-gate — 操作面那一位持全权柄的真客人：把七格验一遍、取回那一枚入口，
//! 再替下一位（**只有 `land` 一位、读不了树**）把两格铺在**根**底下。
//! # 为什么第 5/6 步落在**根**底下
//! 下一位客人只持 `land` 一位 ⇒ 它**问不得** `list` / `seek` / `name`（那三条各是另一柄权），
//! 故它认路的坐标只能自己报得出。**根是唯一不需要号的那一格**（Where::Root：根没有号，
//! 见 operator::frame 那一节）——于是那两格必须落在根底下，下一位才报得出坐标。
//! 这一条是**量出来的**：把两格铺在 `/svc/sys/operator/zone` 底下，那位客人只能列号问名，
//! 于是它每一次 `list` 都被面判拒掉、当场卡死——这正是这一维在按设计生效。
//! # 为什么第 1 步必须在最前
//! 装配者那一步按行 `claim` 本域交出去的孔（有期限 —— `operator::bridge::attach` 的
//! `Wait::AtMost(READY_MS)`），故这一台**不能先做别的手脚再装路**（见
//! `programs/src/harness/probe/probe_bound/main.rs`）。
//! # 为什么它排在整张单的**最前**（`order: Some(3)`）
//! 停机扳机是 `canonical`（那张单上最大 `order` 那一条），而本台问的是**树**（不需要任何驱动）
//! ⇒ 排在身份服务之后、三台驱动之前。**但"窗口"这件事不能靠排队次治**：实测本台自己会走到那两处
//! 重试额度的尽头（旧版是 20 s ＋ 20 s），于是"扳机早于它走完"就变成丢读数——**绿也没有、红
//! 也没有**（喂了输入的 39 跑里丢 20 次；完全不喂的跑里也丢过）。故额度收小到 WAIT_MS、
//! 那七段名字的读数交还给树自己（见 count_under）：数不到就**当场红**。「要读树」的客人
//! 一律排在最前，见 `canonical/program.rs`。

extern crate alloc;
extern crate programs;

use alloc::string::ToString;

use env::Wait;
use programs::Report;
use programs::harness::probe;

use protocol::common::path::Path;
use protocol::communication::session::establish;
use protocol::communication::session::Session;
use protocol::debug;
use protocol::service::operator::client as operator;
use protocol::service::operator::client::{Face as TreeFace, Mine, Pane, Watch};
use protocol::service::operator::{EntryId, Fail, Grant, Permit};
use runtime::env::mail;
use runtime::env::unit as utask;

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

/// **`spot` 那一处**每一拍睡多久（毫秒）：它**不在事件那条路上**——等的是"这一趟走到了"
/// （单槽孔的信），不是树变了，故仍按拍重问（见那一手的注）。
const RETRY_MS: usize = 20;

/// **下一位走完时落的那一格**（`probe-operator-land` 自己落，见那一份的第六步）。
///
/// **它替掉的是从前那一格墙钟**（`HOLD_MS = 1200`）：`claimable` 判"别人有主"那一轴用的判据
/// 是 `vested_by(pie).is_none()`——**主人不在场，那一格就重新可落**，故本台从前必须"压住"
/// 那两格压够下一位走到第四步。压多久是个只能猜的数：短了它测到的是"接手"而不是"拒"
/// （实测红过：`Ok(EntryId(13))`），长了本台自己被停机扳机扑杀、连读数一起丢。
/// 换成**事实**之后两侧都不猜：本台等这条事件才走，下一位落这一格 = "它的判据已经落定"。
const DONE_ROAD: &str = "/probe-op-done";

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）
const OK_NOTE: &str = "probe-operator-gate: seven grants mounted";

/// 声明归本台的那一格（下一位顶它 ⇒ 该拒）。**落在根底下**，见文件头
const OWN: &str = "probe-op-own";
/// 无主的那一格（谁都能落）
const FREE: &str = "probe-op-free";

#[programs::entry]
fn main() -> Report<'static> {
    // 一、**先装路**（次序是硬的，见文件头）。
    let Ok(session) = Session::open(utask::sire(), operator::BERTH, Wait::AtMost(MS)) else {
        panic!("probe-operator-gate: no tree link");
    };
    let tree = TreeFace::of(session);

    // 一·二、**先订**（序是契约）：`Watch::of` 返回就是那个序点——本台此后每一处都是
    //       "先问一次、不满才等事件"，故订阅必须排在"等那一块长出来"之前。
    //       `spot` 那一处不在这条路上：它等的是"这一趟**走到了**"（单槽孔的信），不是树变了。
    //       订要持柄：`watch` 是 `Grant::Watch` 那一维上的一枚（`Face::rein` 借出来），
    //       故这一步同时也在量"这一位拿得到那一柄权"。
    let rein = tree.rein(Grant::Watch);
    let mut watch = match rein.watch(&protocol::service::operator::DIR, Wait::AtMost(MS)) {
        Ok(watch) => watch,
        Err(fail) => panic!("probe-operator-gate: subscribe /svc/sys/operator failed: {fail:?}"),
    };
    // 一·三、**第二条订阅**：下一位落"我走完了"那一格时要收得到。**订在最前**——它可能比本台
    //       走到末尾早（本台中间还有 walk / count / find 三串往返），而订阅之前的改动不在
    //       通知义务内（见 client::Watch 的"序是契约"那一节）。
    let Some(done_road) = protocol::common::path::PathBuf::try_new(DONE_ROAD) else {
        panic!("probe-operator-gate: bad done road");
    };
    let mut done = match rein.watch(&done_road, Wait::AtMost(MS)) {
        Ok(watch) => watch,
        Err(fail) => panic!("probe-operator-gate: subscribe {DONE_ROAD} failed: {fail:?}"),
    };

    // 一·五、**先把那两格摆好**（摆在最前）：下一位客人与本台**并发**跑，而它读不了树
    //       （"那两格摆好了没有"它问不出来）——故本台越早铺，那一条判据越稳。
    //       **铺得比它晚也已经不要紧了**（从前那段"睡 300 ms 再顶"的窗口就是栽在这一处）：
    //       它顶那一格时若还没主，它自己先落下来（`Mine::No` ⇒ 无主），本台这一手随后**换绑**
    //       把归属收过来（`spot` 走的 `bind` 在已占那一格上是换绑，答同一个号）——它下一次再顶
    //       就答 `Denied` 了（见它那一侧的第四步）。
    let own = spot(&tree, OWN, "probe-gate-own", Mine::Yes);
    let free = spot(&tree, FREE, "probe-gate-free", Mine::No);

    let Some(operator_id) = walk(&tree, &protocol::service::operator::DIR, &mut watch) else {
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
    let road = protocol::service::operator::DIR
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

    // 五、那两格已经在（见"一·五"）——这一行只是把读数补齐。
    debug!(
        "probe-operator-gate: land={} cap={} own={} free={}",
        land_id.get(),
        cap.get(),
        own.get(),
        free.get()
    );

    // 六、**压住那两格，直到下一位说它走完了**：`mine = true` 那一轴的判据是"**主人还在不在场**"
    //    （Operator::claimable → `vested_by`），主人一走那一格就重新可落——下一位第四步那一问
    //    当场变成"该通"。**等的是事实**（下一位自己落的那一格推回来的一条事件），不是墙钟：
    //    这一条替掉了从前那一格 `HOLD_MS`（见 `DONE_ROAD` 那一节的注）。
    let told = match done.next(Wait::AtMost(WAIT_MS)) {
        // 收到哪一条不判：第二条订阅只订了那一条路，落在那条路上的任何一条都是"下一位走到了"。
        Ok(ev) => {
            debug!(
                "probe-operator-gate: done kind={:?} id={} seq={}",
                ev.kind,
                ev.id.get(),
                ev.seq
            );
            true
        }
        Err(_) => false,
    };
    if !told {
        // **到点仍没有**：本台照样走完自己的判据（下一位的读数由它那一侧落），但**说一句**，
        // 而且**问清成因**——那一格在不在，是"事件被顶掉"与"对面根本没走到"的分界
        // （那一具架是共享的 `CAP` 格：本台读之前若另有 `CAP` 次改动，手所指的那一格已经换了
        // 内容，订阅那一侧按"路对不上"丢掉 ⇒ 本台等不到，但那一格**在**）。
        let at = tree.tile(&done_road, Wait::POLL);
        protocol::debug::put(&alloc::format!(
            "probe-operator-gate: no done event, cell={}",
            match at {
                Ok(tile) => alloc::format!("present id={}", tile.id().get()),
                Err(fail) => alloc::format!("{fail:?}"),
            }
        ));
    }
    return Report::note(env::EXIT_OK, OK_NOTE);
}

/// `/svc/sys/operator` 那一格自己的号——**一问 ＋ 等事件**：那一块由**别的域**立
/// （本台可能比它先起），"长出来了"那件事就是一条 `Landed`。
/// 三手都是 TreeFace 上现成的手：`root().tile(路)` 译号（**只译号**，不取门闩）、
/// `Tile::id()` 答号、`Tile::pane()` 判"是不是一块 Pane"
fn walk(tree: &TreeFace, road: &Path, watch: &mut Watch<'_>) -> Option<EntryId> {
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

/// 在**根**底下落一格（记号只为本台这台测具而立，不进任何一族的表）
fn spot(tree: &TreeFace, name: &str, mark: &'static str, mine: Mine) -> EntryId {
    let spot = name.to_string();
    let Ok(entry) = mail::unseal_hole(env::Mark::of(mark)) else {
        panic!("probe-operator-gate: no entry");
    };
    // **`Unknown` 重试**（与 `client.rs` 的 `road_to_id` 同一条口径）：那一格由本台与下一位
    // 客人**并发**动，而这一手是"一问一动"——`Unknown` 在这条路上说的是"这一趟没走到"，
    // 不是"这一格不许"。除它以外的失败都是确定的下一步（当场塌）。
    // **它不在事件那条路上**：等的是"这一趟走到了"（单槽孔的信），不是树变了。
    let mut left = WAIT_MS;
    loop {
        match tree
            .root()
            .bind(spot.clone(), entry, Permit::Unset, mine, Wait::AtMost(MS))
        {
            Ok(id) => return id.id(),
            Err(Fail::Unknown) if left > 0 => {
                let _ =
                    runtime::env::room::sleep(core::time::Duration::from_millis(RETRY_MS as u64));
                left = left.saturating_sub(RETRY_MS);
            }
            Err(fail) => panic!("probe-operator-gate: land {name} failed: {fail:?}"),
        }
    }
}
