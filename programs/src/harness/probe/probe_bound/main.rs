#![no_std]
#![no_main]

//! probe-bound — 坏帧的证客：不合族的帧不把门卡死。
//! # 为什么"坏客人"必须是一位真域
//! 判据在核里（`Push` 的长度前置条件），而它的**执行**要一次真 envcall：宿主上没有内核，
//! # 第四条为什么先把它那声 `BAD` 读掉
//! 树的门对**每一条取出来的帧都答一句**（读不懂答 `BAD`）——推了 junk 之后，那声 `BAD` 就落在
//! 本端的树路上。故这一条先把它读掉，再问正经的：留着不清，下一句问会读到上一句的答。
//! **板那一道门同款**（第五条的 `evict` 也先读掉 junk 那一声）。

extern crate alloc;
extern crate programs;

use alloc::string::String;
use alloc::string::ToString;
use env::HoleDir;
use env::Wait;
use env::wire::Field;
use programs::Report;

use env::PieToken;
use protocol::communication::session::establish::Endpoint;
use protocol::communication::session::Session;
use protocol::debug;
use protocol::service::operator as ocall;
use protocol::service::operator::client as operator;
use runtime::env::mail;
use runtime::env::unit as utask;

const MS: usize = 1000;

/// 本地失败编号（读数用）
const E_OK: usize = 0;
const E_TRIP: usize = 1;

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）
const OK_NOTE: &str = "probe-bound: doors held against junk";

const JUNK: usize = 300;

/// 那一条的首格：**第一条动作码**（树那一族的 `LAND` ＝ `1`），其余全零——于是那道门走到
/// "动作认得、形状不对"那一支 ⇒ 答一句 `BAD`
const JUNK_OP: u8 = 1;

/// 编那一条 junk（点数在 JUNK，首格在 JUNK_OP）
fn junk() -> [u8; JUNK] {
    let mut junk = [0u8; JUNK];
    junk[0] = JUNK_OP;
    junk
}

/// **形状全对、只有许可那一格陌生**的那一条 `LAND` 帧（LAND_LEN 字节）
/// 前几格都给合法值：`[0] = LAND`、`[1] = 根`、`[10] = 1`（名字**长度那一字节**）、
/// `[11] = "x"`、`[12..20] = 0`（入口）、`[20] = 0`（`mine` 假）；**唯一越界的是
/// `[21] = 9`**——许可那一张表只有 `0..=3`。故这一条会一路解到许可那一格才断 ⇒
/// **整帧读不懂** ⇒ 门答 `BAD`
/// `[长度那一字节][字节]`（ 里 `String` 的 `Span` impl），故许可那一格落在
/// `1 ＋ 9 ＋ (1 ＋ 名长) ＋ 8 ＋ 1`——**这一台手写裸帧，偏移只能自己数**
/// 表外那一支自己写着"要打到它得造一条这样的 `LAND` 帧，而仓里没有这样一台客人"——上面那条
/// 300 字节的 junk 走 Message::fetch 的长度那一闸，`match` 一次都到不了
fn land_frame(permit_tag: u8) -> [u8; LAND_LEN] {
    let mut f = [0u8; LAND_LEN];
    f[0] = JUNK_OP; // `LAND` ＝ `1`（见 [`JUNK_OP`]）
    f[1] = 0; // `Where::Root`
    f[10] = 1; // 名字长度那一字节
    f[11] = b'x'; // 名字那一个字节
    f[20] = 0; // `mine = false`
    ocall::Permit::Public.store(&mut f[21..]);
    f[21] = permit_tag; // **这一格是唯一要试的那一格**
    f
}

/// `seek` 那一问的动作码：`SEEK = 7`（与 JUNK_OP 同一条：这一台**故意手写裸帧**，故它按
/// 线上那一格写数——`crates/protocol/src/service/operator/frame/vocab.rs` 那一枚私有常量才是正文
/// 那个码要是挪了位，这一条当场红）
const SEEK_OP: u8 = 7;

/// 那一条超长的路：**9 段**（Path::MAX 是 8）
const ROAD_9: &[u8] = b"a/b/c/d/e/f/g/h/i";

/// **一条 9 段的路**（`[op][长度][那些字节]`＝19 字节）——这一台量的是"路太长"那一格挪了家
fn oversize_road() -> [u8; JUNK] {
    let mut road = [0u8; JUNK];
    road[0] = SEEK_OP;
    road[1] = ROAD_9.len() as u8;
    road[2..2 + ROAD_9.len()].copy_from_slice(ROAD_9);
    road
}

/// 表外那一格（`0..=3` 之外）：整帧读不懂
const LAND_PERMIT_UNKNOWN: u8 = 9;
const LAND_PERMIT_KNOWN: u8 = 0;

/// Prefix offset remains a raw-frame acceptance check; Permit width follows its codec.
const LAND_LEN: usize = 21 + ocall::Permit::WIDTH;

#[programs::entry]
fn main() -> Report<'static> {
    let sire = utask::sire();

    // 一、**两条路先都装上**：本端那一枚交给生我者（它再转授给对方），另铸一枚问话孔给它。
    let Ok(session) = Session::open(sire, operator::BERTH, Wait::AtMost(MS)) else {
        return bail("probe-bound: no tree link");
    };
    let (tree, hedge, _) = (&session.link, session.talk, session.host);
    // **正经那一问要一面 `Face`**，而它是**借**一条会话：
    let face = operator::Face::from(&session);
    let Some(dir) = protocol::common::svc::SVC.file_name() else {
        return bail("probe-bound: bad name");
    };

    // 三、往树的门上推一枚不合族的帧，再看那道门还是不是活的。
    let (junk_in, said_bad, after) = junk_trip(hedge, tree, &face, dir.to_string(), &junk());

    let (o_junk_in, o_said_bad, o_after) =
        junk_trip(hedge, tree, &face, dir.to_string(), &oversize_road());

    // 三·三、**形状全对、只有许可那一格陌生**的那一条：同一声 `BAD`（"整帧读不懂"），
    //       门照旧活着。这一条与上一条**不是同一件事**：上一条死在**长度**那一闸，这一条一路
    //       解到许可那一格才断（见 junk_land ）。
    let (l_junk_in, l_said_bad, l_after) = junk_trip(
        hedge,
        tree,
        &face,
        dir.to_string(),
        &land_frame(LAND_PERMIT_UNKNOWN),
    );
    // 不管），故门不该答"读不懂"那一句——两趟并排，才证明上一趟真的断在**许可那一格**上，
    // 而不是断在名字 / `mine` / 长度上。
    let (k_junk_in, k_said_bad, k_after) = junk_trip(
        hedge,
        tree,
        &face,
        dir.to_string(),
        &land_frame(LAND_PERMIT_KNOWN),
    );

    // 四、判据：**一例一条**，名字即结论。
    {
        {
            assert!(junk_in, "不合族的帧推不进门（门那一枚孔不在？）");
            assert!(said_bad, "门没把那一条取出来 / 没答 `BAD`");
            assert!(after, "吞了 junk 之后，门不再答正经的问了");
        }
    }
    {
        {
            assert!(l_junk_in, "那条帧推不进门（门那一枚孔不在？）");
            assert!(
                l_said_bad,
                "许可那一格表外（`[51] = 9`），门该答 `BAD`（整帧读不懂），却没答那一句"
            );
            assert!(l_after, "吞了那条帧之后，门不再答正经的问了");
        }
    }
    {
        {
            assert!(k_junk_in, "差分那条帧推不进门（门那一枚孔不在？）");
            assert!(
                !k_said_bad,
                "只把许可那一格换成表内的 `0`，门仍答 `BAD` ⇒ 上一趟断的不是许可那一格"
            );
            assert!(k_after, "吞了差分那条帧之后，门不再答正经的问了");
        }
    }
    {
        {
            assert!(o_junk_in, "那条 9 段的路推不进门（门那一枚孔不在？）");
            assert!(
                o_said_bad,
                "9 段的路该答 `BAD`（`Path` 里超长根本表达不出来），却没答那一句"
            );
            assert!(o_after, "吞了那条帧之后，门不再答正经的问了");
        }
    }
    programs::harness::probe::identity::timeout();
    return Report::note(E_OK, OK_NOTE);
}

/// 返 `(推成了没有, 读到 BAD 没有, 正经的那一问答得出来没有)`——三格各是一件事，由调用方凑成
/// 一条判据（"这道门没卡死"）
/// 正经那一问取 `part(/sys)`：**幂等**（`/svc` 是服务起手时立的那一格，重复 `part` 只答同一个
/// 号），故"答得出"就是这一条要的全部——答案对不对由别的证客管
/// 只有**紧跟其后那句正经的问**（`part /svc`，幂等）走 `Face`：新面已没有那条自由函数那一层
/// 一个字没变
fn junk_trip(
    hedge: PieToken,
    tree: &Endpoint,
    face: &operator::Face,
    dir: String,
    junk: &[u8],
) -> (bool, bool, bool) {
    let door = mail::HolePie::from_token(hedge);
    let pushed = door.push(junk, Wait::AtMost(MS)).is_ok()
        && matches!(door.wait(HoleDir::Push, Wait::AtMost(MS)), Ok(true));

    // 树路那一枚（本端的读口）：`call` 那份答话就是从它读的。junk 那一声 `BAD` 先读掉。
    let mut back = [0u8; 8];
    let pulled = mail::HolePie::from_token(tree.rx())
        .pull(&mut back, Wait::AtMost(MS))
        .map(|(n, _)| n);
    debug!(
        "probe-bound: junk len={} pull={:?} code={}",
        junk.len(),
        pulled.as_ref().map(|n| *n).map_err(|e| e.source),
        back[0]
    );
    let said = pulled.ok();
    let bad = matches!(said, Some(1) if back[0] == ocall::BAD);

    // 正经的一问：**门还在答**。
    let root = face.root();
    let after = matches!(root.open(dir, Wait::AtMost(MS)), Err(ocall::Fail::Denied));
    (pushed, bad, after)
}

/// 哪里算不下去就报哪一句（kernel 收场时把这一句连同域号打出来）
fn bail<'a>(note: &'a str) -> Report<'a> {
    debug!("{}", note);
    return Report::note(E_TRIP, note);
}
