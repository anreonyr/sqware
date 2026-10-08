#![no_std]
#![no_main]

//! Control publication preserves Watch delivery, filtering and queue behavior.

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

use system_api::operator::path::PathBuf;
use ipc::session::Session;
use programs::debug;
use system_client::operator;
use system_client::operator::Face as Face;
use system_api::operator::Event;
use system_api::operator::Kind;
use system_api::operator::EntryId;
use system_api::operator::Grant;
use system_api::operator::Permit;
use env::unit;
use env::pie;

const MS: usize = 1000;

/// **等事件**的额度（毫秒）：铃响是提示型，收到即醒；给足装配窗口但不做成轮询
const WAIT_MS: usize = 3_000;

/// 订的那条路（树底下一段，够短）
const IN_ROAD: &str = "svc/probe-watch/in";
/// **不在订的范围里**的那一条（第 3 条判据用它）
const OUT_ROAD: &str = "svc/probe-watch/out";

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）
const OK_NOTE: &str = "probe-watch: landed=1 filtered=1";

#[programs::entry]
fn main() -> Report<'static> {
    // 一、**一条会话**（`Face::root()` 那条全操作面的路，与 `probe-rule` 同一手）：
    //    订与落都走它。**一位客人一条会话**是本族的形状（认领键是"谁开的 ＋ 树路记号"），
    //    故这一台不另开第二条——两条会话同开时，第二位客人认不到自己的那条答话路。
    let Ok(session) = Session::open(unit::sire(), operator::BERTH, Wait::AtMost(MS)) else {
        panic!("probe-watch: no tree link");
    };
    let tree = Face::from(&session);

    // 二、**订**：`watch` 那一面回 `OK` 之后，此后真变了才发得过来。
    let road = PathBuf::try_new(IN_ROAD).unwrap_or_else(|| panic!("probe-watch: bad road"));
    // **柄先绑**：`Rein` 是"借这一面借出来的那一柄权"，临时的 `Rein` 活不过这一条绑定。
    let rein = tree.rein(Grant::Watch);
    let mut watch = rein
        .watch(&road, Wait::AtMost(WAIT_MS))
        .unwrap_or_else(|fail| panic!("probe-watch: subscribe refused: {fail:?}"));

    debug!("probe-watch: subscribed road={}", watch.road());

    // 三、**订的范围里落一格** ⇒ 等一条事件（等铃，不轮询）。
    let inside = spot(&tree, IN_ROAD, "probe-watch-in");
    let got = wait_event(&mut watch, "in");
    assert_eq!(got.kind, Kind::Landed, "落下来那一格该报 Landed");
    assert_eq!(
        got.road.as_str(),
        IN_ROAD,
        "事件里那条路不对（收到的是 {}）",
        got.road
    );
    assert_eq!(got.id, inside, "事件里那一号不是刚落的那一格");
    debug!(
        "probe-watch: in id={} road={} kind={:?}",
        got.id.get(),
        got.road,
        got.kind
    );

    // 四、**订的范围外落一格** ⇒ 期限内收不到第二条（过滤）。
    let outside = spot(&tree, OUT_ROAD, "probe-watch-out");
    let quiet = watch.next(Wait::AtMost(300));
    assert!(
        quiet.is_err(),
        "订的那条路之外的改动也发过来了（id={}）：过滤没生效",
        outside.get()
    );
    debug!("probe-watch: out id={} silent=ok", outside.get());

    // 五、**那一页上就这一条**（没有多出来的）：再探一次仍是空。
    let none = watch.try_next();
    assert!(
        matches!(none, Ok(None)),
        "架上多出了没读过的那一条：{none:?}"
    );

    // 七、**队列那一档**：另订一条路（一位订两条路是正当的），在它上面**连落 6 格、中间一条不读**
    //    ——这条路上共 7 条事件（窗格那一下 ＋ 6 格），与环那 7 格正好齐平（**刻意不超**：绕回来就
    //    把格子顶掉，那时读到的是后来的内容，量不到队列本身；第一版连落 8 格就栽在这，`got=0`
    //    还让"读到的比落的少"**空过**）。孔上排得下 `QUEUE_CAP` 只（那是内核的常量，本台不抄它）：
    //    排满之后再来的改动**推不进来**，那几条就丢了（通知不是账）。
    //    判据两条：**读到的比落的少**且**至少读到一条**，而读到的那几条**号严格递增**。
    const LANDED: usize = 6;
    const EVENTS: usize = LANDED + 1;
    let qroad =
        PathBuf::try_new("svc/probe-watch-q").unwrap_or_else(|| panic!("probe-watch: bad q road"));
    let mut queue = rein
        .watch(&qroad, Wait::AtMost(WAIT_MS))
        .unwrap_or_else(|fail| panic!("probe-watch: subscribe /probe-watch/q refused: {fail:?}"));
    for i in 0..LANDED {
        let _ = spot(
            &tree,
            &alloc::format!("svc/probe-watch-q/c{i}"),
            "probe-watch-q",
        );
    }
    let mut got = 0usize;
    let mut last = 0u64;
    while let Ok(Some(ev)) = queue.try_next() {
        assert!(
            ev.seq > last,
            "读到的号没递增（{last} → {}）：那一列手没按先进先出给",
            ev.seq
        );
        last = ev.seq;
        got += 1;
    }
    assert!(
        got > 0 && got < EVENTS,
        "队列那一条没量到：这条路上 {EVENTS} 条事件、读到 {got} 条（0 = 一条没收到，= {EVENTS} = 一条没丢）"
    );
    // **读数**（`debug!` 在 release 是空操作，故这一行走 `debug::put`）。
    programs::debug::put(&alloc::format!(
        "probe-watch: queued got={got} of={EVENTS} last_seq={last}"
    ));

    return Report::note(env::EXIT_OK, OK_NOTE);
}

/// **等一条事件**（等到就返；额度走完就当场红——"没到"与"到错了"都是红）。
fn wait_event(watch: &mut operator::Watch<'_>, what: &str) -> Event {
    if let Ok(Some(got)) = watch.try_next() {
        return got;
    }
    match watch.next(Wait::AtMost(WAIT_MS)) {
        Ok(got) => got,
        Err(fail) => panic!("probe-watch: {what} 那条事件没等到（{WAIT_MS} ms，{fail:?}）"),
    }
}

/// 在**根**底下按整条路落一格，答它自己的号。
///
/// 前缀那几段由**本台自己分**（`Pane::open`：幂等，缺的就地造）——那几段是这一台自己的试验场，
/// 不是别人的。**逐段只收号**（`Pane` 借的是上一块，攒着它就没法在循环里换新的）；
/// 落完再沿整条路译一遍，确认"事件里那个号"与树上那一格是同一个。
fn spot(tree: &Face, road: &str, mark: &'static str) -> EntryId {
    let (parent, name) = road.rsplit_once('/').unwrap();
    let group = parent.strip_prefix("svc/").unwrap();
    let entry = pie::unseal_hole(env::Mark::of(mark)).unwrap();
    let target = system_api::control::publication::Target::Service {
        scope: system_api::control::publication::Scope::Fixture,
        group: group.into(),
        name: name.into(),
    };
    let id = system_client::control::publication::Client::injected()
        .unwrap()
        .publish(target, entry, Permit::Public, Wait::AtMost(MS))
        .unwrap();
    assert_eq!(
        tree.root()
            .tile(&PathBuf::try_new(road).unwrap(), Wait::AtMost(MS))
            .unwrap()
            .id(),
        id
    );
    id
}
