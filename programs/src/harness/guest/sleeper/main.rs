#![no_std]
#![no_main]

//! sleeper — **客人**：问一声现在几点、约一个时刻、**睡到那一声**、走人。
//!
//! 它是 `rtc` 那台驱动的**真客人**（`/svc/drv/rtc` 那块门牌第一位用家）：那台时钟只有持有它的域
//! 读得动（`ONLY`），故"报时 / 定闹钟"这两件事只能由驱动替它做——本域说两句话、收两句话。
//!
//! # 失败域那一趟是有意的
//!
//! 与 `lodger` 三趟同一条纪律：**失败域也要卖出读数**，而那两格拿本域自己的线试是**确定**的
//! ——"过去的时刻"由本域自己算得出来（`now` 是刚问的），"那一格有人了"就是本域刚约下的那一次
//! （换别人试会与它抢时间，那是竞态不是读数）。
//!
//! # 为什么它不碰设备
//!
//! 那台时钟归驱动持有（`ONLY` 是资源事实）。本域**一枚门闩都不要**（装配表里 `setup` 是空的）
//! ——它只是那一面服务的第一位客人：拿得到的是树上那枚门牌孔的副本，不是设备。
//!
//! # 特权级
//!
//! **U 态**（`programs::unit::PROGRAMS` 里这一行的 `kind`）：铸孔、交孔、上树找服务、一问一答都不需要 S 态。

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

// 树：本域是**客侧**（按名找服务）；板：也是客侧（只为让板看见本域的死）。
use protocol::communication::session::Session;
use protocol::debug;
use protocol::service::operator::Fail;
use protocol::service::operator::client as operator;
use protocol::service::operator::client::Face;

use env::PieToken;
// 那一面服务：帧形与记号、客侧两手——**与驱动同一份源码**（见 `programs/src/driver/rtc/mod.rs`）。
use programs::driver::rtc::client as clock;
use programs::driver::rtc::core::Fail as RFail;
use programs::driver::rtc::core::frame as rcall;
use runtime::env::unit as utask;

/// 本域挂在板上的名字（板按它分人；编排域表里那一条也叫这个）。
const ME: &str = "sleeper";

/// 要找的那位服务在树上的名字：**实时钟**（`/svc/drv/rtc`——名字用服务名）。
const WANT: &str = "rtc";

/// 等板 / 等树 / 找一趟服务 / 办一趟往返的总上限（毫秒）。**必须有界**。
const MS: usize = 1000;

/// 这一槽的**周期**（纳秒）："再过这么久叫我"。`Wire::Arm` 收了相对量之后，这个数就是
/// **想要的那段距离本身**，不再是"要罩住一趟往返的提前量"——延迟由收帧的驱动承担
/// （见 `programs/src/driver/rtc/core/frame.rs` 那格），故它不必再留 4× 余量。
const SLOT_NS: u64 = 50_000_000;

/// 没搭上（找不到那面服务 / 有一条往返没走成）：报这一格退场。
const E_NO_SERVICE: usize = 1;

/// 没搭上：**报码 ＋ 指名是哪一步**。
fn no_service(step: &'static str) -> Report<'static> {
    Report::note(E_NO_SERVICE, step)
}

#[programs::entry]
fn main() -> Report<'static> {

    let sire = utask::sire();
    let Ok(session) = Session::open(sire, operator::BERTH, Wait::AtMost(MS)) else {
        return no_service("sleeper: no operator");
    };
    let tree = Face::of(session);
    let Some(face) = find_face(&tree) else {
        return no_service("sleeper: no rtc plate");
    };
    debug!("sleeper: found");

    // 一问一答：现在几点。这一句是后面那两约的**基准**（服务收的是绝对时刻）。
    let Ok(now) = clock::now(face, Wait::AtMost(MS)) else {
        return no_service("sleeper: no time");
    };
    debug!("sleeper: now={now}");


    // 真约：那一枚回信孔从此留在驱动手里（本域退场之前它一直活着）。
    let armed = match clock::arm(face, SLOT_NS, Wait::AtMost(MS)) {
        Ok(alarm) => alarm,
        Err(fail) => {
            debug!("sleeper: alarm err={}", rcall::fail_to_code(Some(fail)));
            return no_service("sleeper: no alarm");
        }
    };
    debug!("sleeper: armed={}", rcall::fail_to_code(None));

    // 失败域第二格：再约一次。那一格里有人——就是本域刚约下的那一次（拿自己的线试，
    // 答 `TAKEN` 是确定的）。
    let taken = refused(clock::arm(face, SLOT_NS, Wait::AtMost(MS)));
    debug!("sleeper: taken={taken}");

    // 等到那一声：**无界等**（本域只有这一件事），而对面一没那枚孔就封印、当场答错。
    let Ok(rang) = armed.receive() else {
        return no_service("sleeper: no ring");
    };
    debug!("sleeper: rang after={SLOT_NS} now={rang}");

    // 判据就地登记：**只搬本域已经在判的东西**。三例的期望都是
    // 本站此刻就知道的，而且比较用的是**与读数同一批常量**（`bcall::OK` / `rcall::` 那两个码），
    // 不是新写死的数字。
    {
        assert_eq!(taken, rcall::TAKEN)
    }

    return Report::note(env::EXIT_OK, "sleeper: gone");
}

/// 被拒那一趟的读数：把失败域按**线上那张表**折成一个数（与驱动的答码同源）。
fn refused(result: Result<clock::Alarm, RFail>) -> u8 {
    match result {
        Ok(_) => rcall::fail_to_code(None),
        Err(fail) => rcall::fail_to_code(Some(fail)),
    }
}

/// 找那面服务：`FIND /svc/drv/rtc`，**找不到就再问**（有界）——门牌是驱动落的，本域可能比它先起。
///
/// 找到之后那一枚**从会话里**进本域表（报文里没有号）：认的是"持树者刚授进来的那一份"，
/// 而本域此刻只查了这一趟 ⇒ 这一趟拿走的一定是它。
fn find_face(tree: &Face) -> Option<PieToken> {
    let road = protocol::driver::ROAD.try_join(WANT)?;
    // 名字 → 号（**译不出就重试**：门牌是驱动落的，它可能落得比本域晚）→ 入口：两格在
    // [`Pane::tile`] 与 [`Tile::token`] 上。
    let root = tree.root();
    let mut left = MS;
    loop {
        match root
            .tile(&road, Wait::AtMost(MS))
            .and_then(|entry| entry.token(Wait::AtMost(MS)))
        {
            Ok(entry) => return Some(entry),
            Err(Fail::Unknown) if left > 0 => {
                let _ = runtime::env::room::sleep(core::time::Duration::from_millis(1));
                left = left.saturating_sub(1);
            }
            Err(_) => return None,
        }
    }
}
