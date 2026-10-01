#![no_std]
#![no_main]

//! # 为什么两条路都走
//! （protocol::driver::ROAD 那段目录）。
//! # 一问一答由这两趟各自证
//! 自己的客人（`lodger`：占一条线就死、失败那趟也走一遍）。
//! # 特权级由清单定

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

use env::PieToken;
use protocol::communication::session::Session;
use protocol::debug;
use protocol::service::operator as ocall;
use protocol::service::operator::Fail;
use protocol::service::operator::client as operator;
use protocol::service::operator::client::Face;
use runtime::env::unit as utask;

const ME: &str = "guest";
const WANT: &str = "router";

const MS: usize = 1000;

const E_OK: usize = 0;
const E_TRIP: usize = 1;

#[programs::entry]
fn main() -> Report<'static> {
    let sire = utask::sire();
    let none = PieToken::NONE;

    // 一、与树开会话：本端那一枚交给生我者（它再转授给持树者），另铸一枚问话孔给它。
    let Ok(session) = Session::open(sire, operator::BERTH, Wait::AtMost(MS)) else {
        return bail("guest: no tree link");
    };
    let tree = Face::of(session);
    let Some(road) = protocol::driver::ROAD.try_join(WANT) else {
        return bail("guest: bad name");
    };

    // **间接寻址那一手**：名字先译成号（那一格才谈得上"挂上了没有"），拿到号再按号寻。
    // `find` 那一族的失败折成失败域那一格（按本族那张表折回数：
    // **`BAD` / `UNKNOWN` / 没走到是同一格** Fail::Unknown）。
    // `AtMost(MS)` 是**额度不是整趟时限**（往返耗时不计账、推不进去还会等在门外）。
    let (find, at) = match find_face(&tree, &road) {
        Ok(entry) => (ocall::OK, entry),
        Err(fail) => (ocall::fail_to_code(Some(fail)), none),
    };

    debug!("guest: find={find} entry={}", at.get());

    // 此刻就知道的期望。
    {
        assert_eq!(find, ocall::OK)
    }
    {
        {
            assert!(at != none);
        }
    }

    let walked = find == ocall::OK && at != none;
    return Report::note(
        if walked { E_OK } else { E_TRIP },
        if walked {
            "guest: trip ok"
        } else {
            "guest: trip failed"
        },
    );
}

fn find_face(tree: &Face, road: &protocol::service::operator::Path) -> Result<PieToken, Fail> {
    let root = tree.root();
    let mut left = MS;
    loop {
        match root
            .tile(road, Wait::AtMost(MS))
            .and_then(|entry| entry.token(Wait::AtMost(MS)))
        {
            Ok(entry) => return Ok(entry),
            Err(Fail::Unknown) if left > 0 => {
                let _ = runtime::env::room::sleep(core::time::Duration::from_millis(1));
                left = left.saturating_sub(1);
            }
            Err(fail) => return Err(fail),
        }
    }
}

/// 哪里算不下去就报哪一句（kernel 收场时把这一句连同域号打出来）
fn bail<'a>(note: &'a str) -> Report<'a> {
    return Report::note(E_TRIP, note);
}
