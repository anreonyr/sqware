#![no_std]
#![no_main]

//! 验证 Control 查询面可访问，受限写入面拒绝未授权调用。
//! 通道先就绪；请求等待 Control 进入监督循环。失败以 panic 报告给整机验收。

extern crate alloc;
extern crate programs;

use alloc::string::ToString;

use env::Wait;
use programs::Report;
use programs::harness::probe;

use protocol::communication::session::Session;
use protocol::debug;
use protocol::system::control as ccall;
use protocol::system::operator::Fail as TreeFail;
use protocol::system::operator::Grant as TreeGrant;
use protocol::system::operator::client as operator;
use runtime::env::unit as utask;

const MS: usize = 1000;

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）
const OK_NOTE: &str = "probe-control: ask open, three faces denied";

/// 等待 Control 开始处理请求的总窗口（毫秒）。
/// （`probe-control` 的头注：第 4 / 5 步都要等）——故有界：等的是**事件**（`Watch::next`），
/// 到点由下面的 `assert_eq!` 落地。
const FACES_MS: usize = 3_000;

/// **一个一定不在装配表里的名字**：第 4 步那一问的荷载
/// 取"表里没有"是**故意**的：那一问要的就是"对面答得出一句语义码"。若拿一个真名字去问
/// 答案会跟着那一台的生命阶段变（同一句话量出两件事）
const NOBODY: &str = "probe-control-nobody";

#[programs::entry]
fn main() -> Report<'static> {
    // 一、**先装路**（次序是硬的，见文件头）。
    let Ok(session) = Session::open(utask::sire(), operator::BERTH, Wait::AtMost(MS)) else {
        panic!("probe-control: no tree link");
    };
    let tree = operator::Face::from(&session);

    // 一·五、**先订**（序是契约）：那一族此后每落一面都往本端这一页记一条，`Watch::of`
    //       返回就是那个序点——已经落齐的情形由量具第一问当场返回，不必等事件。
    //       订要持柄：`watch` 是 `Grant::Watch` 那一维上的一枚（`Face::rein` 借出来）。
    let rein = tree.rein(TreeGrant::Watch);
    let mut watch = match rein.watch(ccall::DIR, Wait::AtMost(MS)) {
        Ok(watch) => watch,
        Err(fail) => panic!("probe-control: /svc/sys/control 那一族订不成：{fail:?}"),
    };

    // 二、树上那**一格**：名字（`seek`）→ 号 → **门牌那一枚**（`find` 会把它授进本表）。
    // 那段前缀（一块 `Pane`，没有门牌可授）。
    let road = ccall::DIR
        .try_join(ccall::Grant::State.name())
        .expect("probe-control: bad name");

    // 二之后、三之前：**那一族到齐没有**——一问 ＋ 等事件（**该有几枚 = `Grant::ALL`**：
    // 一枚 Grant = 一枚门牌 = 一格）。与 `probe_operator_gate` 数 `/svc/sys/operator` 同一把尺子。
    let parent = tree
        .pane(ccall::DIR, Wait::AtMost(MS))
        .unwrap_or_else(|fail| panic!("probe-control: /svc/sys/control is not a pane: {fail:?}"));
    let seen =
        probe::count::count_under(&parent, ccall::Grant::ALL.len() + 1, &mut watch, FACES_MS);
    debug!(
        "probe-control: faces={seen} want={}",
        (ccall::Grant::ALL.len() + 1)
    );
    assert_eq!(
        seen,
        (ccall::Grant::ALL.len() + 1),
        "/svc/sys/control 底下不对齐（Grant::ALL 有 {} 枚，数到的只有 {seen} 格）",
        (ccall::Grant::ALL.len() + 1)
    );
    let plate = tree
        .tile(&road, Wait::AtMost(MS))
        .and_then(|tile| tile.token(Wait::AtMost(MS)));
    let Ok(entry) = plate else {
        panic!("probe-control: /svc/sys/control/state is not on the tree");
    };

    let Ok(control) = ccall::Face::of(entry) else {
        panic!("probe-control: plate refused as a control face");
    };

    // 四、**表里没有的名字** ⇒ 期望那一格语义码（`Unknown`）。这一条量的正是"这一问走到了对面、
    let nobody = NOBODY.to_string();
    let missing = control.service(nobody.clone()).state(Wait::AtMost(MS));
    debug!("probe-control: missing={missing:?}");

    // 五、**本台自己**：装配表里有这一行、而本台此刻活着 ⇒ 答得出一个生命阶段。
    let mine = "probe-control".to_string();
    let alive = control.service(mine).state(Wait::AtMost(MS));
    debug!("probe-control: self={alive:?}");

    // 六、**带规矩那四面**：`mint` / `embark` / `debark` / `ruin` 各取一遍 ⇒ 期望 `Denied`（树那一层）。
    let mut denied_cells = [false; 4];
    for (i, grant) in [
        ccall::Grant::Mint,
        ccall::Grant::Embark,
        ccall::Grant::Debark,
        ccall::Grant::Ruin,
    ]
    .into_iter()
    .enumerate()
    {
        let road = ccall::DIR
            .try_join(grant.name())
            .expect("probe-control: bad face name");
        let got = tree
            .tile(&road, Wait::AtMost(MS))
            .and_then(|tile| tile.token(Wait::AtMost(MS)));
        debug!("probe-control: {}=err:{got:?}", grant.name());
        denied_cells[i] = matches!(got, Err(TreeFail::Denied));
    }

    //     走到了对面，是对面**说得清清楚楚**地拒的）。
    let write_mint = control.mint(nobody.clone(), Wait::AtMost(MS)).err();
    let write_start = control
        .service(nobody.clone())
        .embark(Wait::AtMost(MS))
        .err();
    let write_stop = control
        .service(nobody.clone())
        .debark(Wait::AtMost(MS))
        .err();
    debug!(
        "probe-control: mint(ask)={write_mint:?} start(ask)={write_start:?} stop(ask)={write_stop:?}"
    );

    let write_ruin = control.service(nobody).ruin(Wait::AtMost(MS)).err();
    assert!(
        matches!(write_ruin, Some(ccall::Fail::Denied)),
        "问面发不出 Ruin：{write_ruin:?}"
    );

    // 八、判据：**一例一条**，名字即结论（`Bad` 那一格在每一条里都是红）。
    assert!(
        matches!(missing, Err(ccall::Fail::Unknown)),
        "control 没答出「表里没这个名字」：{missing:?}（`Bad` = 这一趟没走到对面）"
    );
    assert!(
        matches!(alive, Ok(ccall::State::Ready)),
        "control 那一面看不见本台（装配表里的一行）：{alive:?}"
    );
    for (i, grant) in [
        ccall::Grant::Mint,
        ccall::Grant::Embark,
        ccall::Grant::Debark,
        ccall::Grant::Ruin,
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            denied_cells[i],
            "{} 那一格带了规矩（许给开着这一格的那位），本台该取不回",
            grant.name()
        );
    }
    assert!(
        matches!(write_mint, Some(ccall::Fail::Denied)),
        "问面发不出 Mint：{write_mint:?}（`Bad` = 这一趟没走到对面）"
    );
    assert!(
        matches!(write_start, Some(ccall::Fail::Denied)),
        "问面发不出 Embark：{write_start:?}"
    );
    assert!(
        matches!(write_stop, Some(ccall::Fail::Denied)),
        "问面发不出 Debark：{write_stop:?}"
    );
    return Report::note(env::EXIT_OK, OK_NOTE);
}
