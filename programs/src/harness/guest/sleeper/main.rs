#![no_std]
#![no_main]

//! 问一声现在几点、约一个时刻、睡到那一声、走人。
//! 它是 `rtc` 那台驱动的**真客人**（`/svc/drv/rtc` 那块门牌第一位用家）：那台时钟只有持有它的域
//! （换别人试会与它抢时间，那是竞态不是读数）。
//! # 为什么它不碰设备
//! # 特权级
//! **U 态**（programs::unit::PROGRAMS 里这一行的 `kind`）：铸孔、交孔、上树找服务、一问一答都不需要 S 态。

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

use protocol::communication::session::Session;
use protocol::debug;
use protocol::system::operator::{Fail, Face};
use protocol::system::operator::client as operator;

use env::PieToken;
use programs::driver::rtc::client as clock;
use programs::driver::rtc::core::Fail as RFail;
use programs::driver::rtc::core::frame as rcall;
use env::unit;

/// 要找的那位服务在树上的名字：**实时钟**（`/svc/drv/rtc`——名字用服务名）
const WANT: &str = "rtc";

/// 等板 / 等树 / 找一趟服务 / 办一趟往返的总上限（毫秒）。**必须有界**
const MS: usize = 1000;

/// 这一槽的**周期**（纳秒）："再过这么久叫我"。Wire::Arm 收了相对量之后，这个数就是
/// **想要的那段距离本身**，不再是"要罩住一趟往返的提前量"——延迟由收帧的驱动承担
/// （见 `programs/src/driver/rtc/core/frame.rs` 那格），故它不必再留 4× 余量
const SLOT_NS: u64 = 50_000_000;

const E_NO_SERVICE: usize = 1;

/// 没搭上：**报码 ＋ 指名是哪一步**
fn no_service(step: &'static str) -> Report<'static> {
    Report::note(E_NO_SERVICE, step)
}

#[programs::entry]
fn main() -> Report<'static> {
    let sire = unit::sire();
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

    let armed = match clock::arm(face, SLOT_NS, Wait::AtMost(MS)) {
        Ok(alarm) => alarm,
        Err(fail) => {
            debug!("sleeper: alarm err={}", rcall::fail_to_code(Some(fail)));
            return no_service("sleeper: no alarm");
        }
    };
    debug!("sleeper: armed={}", rcall::fail_to_code(None));

    // 答 `TAKEN` 是确定的）。
    let taken = refused(clock::arm(face, SLOT_NS, Wait::AtMost(MS)));
    debug!("sleeper: taken={taken}");

    let Ok(rang) = armed.receive() else {
        return no_service("sleeper: no ring");
    };
    debug!("sleeper: rang after={SLOT_NS} now={rang}");

    // 本站此刻就知道的，而且比较用的是**与读数同一批常量**（bcall::OK / rcall:: 那两个码），
    // 不是新写死的数字。
    {
        assert_eq!(taken, rcall::TAKEN)
    }

    return Report::note(env::EXIT_OK, "sleeper: gone");
}

fn refused(result: Result<clock::Alarm, RFail>) -> u8 {
    match result {
        Ok(_) => rcall::fail_to_code(None),
        Err(fail) => rcall::fail_to_code(Some(fail)),
    }
}

fn find_face(tree: &Face) -> Option<PieToken> {
    let road = protocol::driver::ROAD.try_join(WANT)?;
    // Pane::tile 与 Tile::token 上。
    let root = tree.root();
    let mut left = MS;
    loop {
        match root
            .tile(&road, Wait::AtMost(MS))
            .and_then(|entry| entry.token(Wait::AtMost(MS)))
        {
            Ok(entry) => return Some(entry),
            Err(Fail::Unknown) if left > 0 => {
                let _ = execution::room::park(core::time::Duration::from_millis(1));
                left = left.saturating_sub(1);
            }
            Err(_) => return None,
        }
    }
}
