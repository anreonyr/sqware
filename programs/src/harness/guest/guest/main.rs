#![no_std]
#![no_main]

//! guest — **第一位真客人**：按名字找到一个服务，走完一趟就退场。
//!
//! 本域手里只有一样东西：**名字**。`router` 在哪个域、哪一枚孔、谁建的——那三样由**树**回答
//! （`FIND /svc/drv/router` 把入口**经会话**授进本域表里，不从报文里来）；**板**那边本域只用
//! 本域今天只走树上那一趟（按名找人）；板的死信号已整片退场，本域不再"上板／EVICT"。
//!
//! ```text
//!   1  板那条路：seat(板) + claim(生我者, 板) —— 本端那一枚孔交给生我者（装答话路）；
//!      另铸一枚**问话孔**给板
//!   2  REGISTER "guest"：本域的服务入口经会话交给板（于是本域也能被按名字找到）
//!   3  树那条路：seat(树) + claim(生我者, 树)，另铸一枚问话孔给持树者
//!   4  FIND "/svc/drv/router"：树上问一句，入口从会话里进本域表（找不到就再问，有界）
//!   5  说一句 EVICT（**一字节帧**）——"我走了"：板据此撤格 + 摘掉本域挂在板上的牌子
//!   6  报一行读数就退场 —— 一次往返，不留常驻
//! ```
//!
//! # 为什么两条路都走
//!
//! **按名找服务走树**（用户裁定：驱动挂 `/svc/drv`）。故"找 `router`"全走树
//! （[`protocol::driver::ROAD`] 那段目录）。
//!
//! # 一问一答由这两趟各自证
//!
//! 本域不再跟谁说"招呼"：它证的"一问一答"落在板与树那两趟上——两趟都是"本域出一句、
//! 另一个域回一格码"（读数里的 `reg=` / `find=`）。线那一面（登记 / 投递 / 排空）归线协议
//! 自己的客人（`lodger`：占一条线就死、失败那趟也走一遍）。
//!
//! # 特权级由清单定
//!
//! 本域是 **U 态**（`programs::unit::PROGRAMS` 里这一行的 `kind`）：铸孔、交出、一问一答**都不需要 S 态**，
//! 故一个最小特权的域也能按名字找到服务——这一刀最想验的就是这一句。（唯一收在 S 态的是
//! 铸**门铃**，本域用不着。）
//!
//! **退场**：本域走完那一趟，`main` 退场（kernel 收场时把 `guest: …` 那一行连同域号打出来）。

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

// 板：本域是**客侧**（挂牌子、说一句"我走了"）；树：本域也是客侧（按名找人）。
use env::PieToken;
use protocol::communication::session::Session;
use protocol::debug;
use protocol::service::operator as ocall;
use protocol::service::operator::Fail;
use protocol::service::operator::client as operator;
use protocol::service::operator::client::Face;
use runtime::env::unit as utask;

/// 本域挂在板上的名字，与要找的那个服务——**本域知道的全部**。
const ME: &str = "guest";
const WANT: &str = "router";

/// 等板 / 等答的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 两种退场：走通了 / 没走通（都**不是 panic**；kernel 会把那一行连同域号打出来）。
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

    // 三、问一句名字。**找不到就再问**，有界：本域可能比 `router` 先起（树上没有"装配期"）。
    // **间接寻址那一手**：名字先译成号（那一格才谈得上"挂上了没有"），拿到号再按号寻。
    //
    // 那一趟（含"译不出就重试"）在 [`find_face`] 里：答话码原样往外带，
    // `find` 那一族的失败折成失败域那一格（按本族那张表折回数：
    // **`BAD` / `UNKNOWN` / 没走到是同一格** `Fail::Unknown`）。
    // `AtMost(MS)` 是**额度不是整趟时限**（往返耗时不计账、推不进去还会等在门外）。
    let (find, at) = match find_face(&tree, &road) {
        Ok(entry) => (ocall::OK, entry),
        Err(fail) => (ocall::fail_to_code(Some(fail)), none),
    };

    // 二、查到的那一枚（持树者经会话授进本域表里）：本域在表里认得出它吗（读数里的 `entry`）
    // ——它就是上面那一趟带回来的号。
    debug!("guest: find={find} entry={}", at.get());

    // 判据就地登记：**只搬本域已经在判的东西**——那两样都是本站
    // 此刻就知道的期望。
    {
        assert_eq!(find, ocall::OK)
    }
    {
        {
            assert!(at != none);
        }
    }

    // 六、退场：一次往返，不留常驻（kernel 打的那一行就是这一格的读数）。
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

/// 沿一条路取那一枚入口（旧 `Face::tile` 那一趟）：**译不出就重试**（有界——门牌是别的域
/// 落的，本域可能比它先起），译出来再要那一枚。
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

/// 哪里算不下去就报哪一句（kernel 收场时把这一句连同域号打出来）。
fn bail<'a>(note: &'a str) -> Report<'a> {
    return Report::note(E_TRIP, note);
}
