#![no_std]
#![no_main]

//! guest — **第一位真客人**：按名字找到一个服务，走完一趟就退场。
//!
//! 本域手里只有一样东西：**名字**。`router` 在哪个域、哪一枚孔、谁建的——那三样由**树**回答
//! （`FIND /device/router` 把入口**经会话**授进本域表里，不从报文里来）；**板**那边本域只用
//! 两格：挂上自己的牌子（`REGISTER`）与退场那句 `EVICT`。
//!
//! ```text
//!   1  板那条路：seat(板) + claim(生我者, 板) —— 本端那一枚孔交给生我者（装答话路）；
//!      另铸一枚**问话孔**给板
//!   2  REGISTER "guest"：本域的服务入口经会话交给板（于是本域也能被按名字找到）
//!   3  树那条路：seat(树) + claim(生我者, 树)，另铸一枚问话孔给持树者
//!   4  FIND "/device/router"：树上问一句，入口从会话里进本域表（找不到就再问，有界）
//!   5  说一句 EVICT（**一字节帧**）——"我走了"：板据此撤格 + 摘掉本域挂在板上的牌子
//!   6  报一行读数就退场 —— 一次往返，不留常驻
//! ```
//!
//! # 为什么两条路都走
//!
//! **按名找服务归树，板管生死**（用户裁定：驱动挂 `/device`）。故"找 `router`"走树
//! （[`protocol::driver::DIR`] 那段目录），而自己那块牌子仍挂板：板那一侧的 `EVICT`
//! （客人自己说走）只有本域在用。**照实记**：板上的 `LOOKUP` 从此**不会有真客人**——命名归树
//! （见 `protocol::system::board` 那一格照实记）。
//!
//! # 一问一答由这两趟各自证
//!
//! 本域不再跟谁说"招呼"：它证的"一问一答"落在板与树那两趟上——两趟都是"本域出一句、
//! 另一个域回一格码"（读数里的 `reg=` / `find=`）。线那一面（登记 / 投递 / 排空）归线协议
//! 自己的客人（`lodger`：占一条线就死、失败那趟也走一遍）。
//!
//! **照实记**：从前这里还写着"两个方向各 32 字节、与牌子同一个解码面"——那一形状（本域推
//! 名字进 `router` 的门、它回自己的名字）与"门后只有登记"相冲，用户裁定之后整条退休：
//! 找人走树，门后只剩带动作码的那一种帧。
//!
//! # 特权级由清单定
//!
//! 本域是 **U 态**（`plan::assembly::ALL` 里这一行的 `kind`）：铸孔、交出、一问一答**都不需要 S 态**，
//! 故一个最小特权的域也能按名字找到服务——这一刀最想验的就是这一句。（唯一收在 S 态的是
//! 铸**门铃**，本域用不着。）
//!
//! **退场**：本域**自己说**一句 `EVICT`（一字节帧）——板据此撤掉本域那一格、摘掉本域
//! 挂在板上的全部牌子、并把本域的问话孔从组里摘掉（名字的位置留着）。**没说就走**的那种
//! 仍由板**看见**（答话路那枚孔径死）后惰性剔除。

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

// 板：本域是**客侧**（挂牌子、说一句"我走了"）；树：本域也是客侧（按名找人）。
use protocol::system::board::client as board;
use protocol::system::operator as ocall;
use protocol::system::operator::client as operator;

use alloc::format;
use core::time::Duration;

use env::{Name, PieToken};
use protocol::system::board as bcall;
use runtime::env::debug;
use runtime::env::mail;
use runtime::env::room;
use runtime::env::unit as utask;

/// 本域挂在板上的名字，与要找的那个服务——**本域知道的全部**。
const ME: &str = "guest";
const WANT: &str = "router";

/// 等板 / 等答的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 找不到就再问一次的间隔（毫秒）：板是**运行期**的账，本域可能比 `router` 先起。
const RETRY_MS: usize = 1;

/// 本地失败写进读数的那一格（与 `board::BAD` 同值：没走到 / 读不懂）。
const BAD: u8 = bcall::BAD;

/// 两种退场：走通了 / 没走通（都**不是 panic**；kernel 会把那一行连同域号打出来）。
const E_OK: usize = 0;
const E_TRIP: usize = 1;

#[programs::entry]
fn main() -> Report<'static> {
    let Ok(sire) = utask::sire() else {
        return bail("guest: no sire");
    };
    // 板那条路：本端装一条、认下生我者那一枚（孔交给生我者，它再转授给板线程）。
    //
    // **必须先于铸入口**：入口与问话孔都是本端铸的、都交到板手里，而板按**记号**分人
    // ——牌子这一格只认得"entry"那一枚；两枚同来源的孔若不刻记号，板就分不出哪个是入口。
    let Ok((link, board)) = board::open(sire, Wait::AtMost(MS)) else {
        return bail("guest: no board link");
    };
    // 问话孔：本端铸、给板读（本端自窄到只写）——问话从它走，答话走上面那条板路。
    // `board` = 板路上先到的那一格（**答话的是谁**）：孔只在铸它的表里念得出来，故这个号
    // 是"板收得到问话"的前提。
    let Ok(talk) = board::ask_hole(board) else {
        return bail("guest: no ask hole");
    };
    // 本域的服务入口：别人按名字找到本域之后往它说话，本域从它读。它也是要交给板的那一枚
    // ——记号 `entry`：板那侧按它把入口与问话孔分开（两枚都是本端铸、本端交）。
    let Ok(entry) = mail::unseal_hole(board::ENTRY_MARK) else {
        return bail("guest: no entry");
    };
    let Ok(me) = Name::new(ME) else {
        return bail("guest: bad name");
    };
    let Ok(want) = Name::new(WANT) else {
        return bail("guest: bad name");
    };
    let none = PieToken::NONE;

    // 一、挂上自己：服务入口经会话交给板（板因此答得出"guest 在哪"）。
    let reg = board::register(talk, &link, board, me, entry, Wait::AtMost(MS)).unwrap_or(BAD);

    // 二、与树开会话：本端那一枚交给生我者（它再转授给持树者），另铸一枚问话孔给它。
    let Ok((tree, host)) = operator::open(sire, Wait::AtMost(MS)) else {
        return bail("guest: no tree link");
    };
    let Ok(hedge) = operator::ask_hole(host) else {
        return bail("guest: no tree ask");
    };
    let Ok(dir) = Name::new(protocol::driver::DIR) else {
        return bail("guest: bad name");
    };
    let path = [dir, want];

    // 三、问一句名字。**找不到就再问**，有界：本域可能比 `router` 先起（树上没有"装配期"）。
    // **间接寻址那一手**：名字先译成号（那一格才谈得上"挂上了没有"），拿到号再按号寻。
    //
    // **照实记（乙′：这一格从"两趟"并成"一趟"）**：`find` 从前只答一格状态，查到的那一枚要
    // 另叫一手 `operator::take` 扫本域表按"谁给的"认回来。今天那一枚号**随答话回来**，故
    // 这一趟连号带状态一起破出去；`at` 就是本域表里那一枚（读数里的 `entry`）。
    let mut left = MS;
    let (find, at) = loop {
        match operator::seek(hedge, &tree, &path, Wait::AtMost(MS)) {
            // **树那一问用树自己的码**（`ocall::BAD` = 7；`BAD` 那一格是板那一面的，值 5）。
            Ok(id) => match operator::find(hedge, &tree, id, Wait::AtMost(MS)) {
                Ok((code, entry)) => break (code, entry.unwrap_or(none)),
                Err(_) => break (ocall::BAD, none),
            },
            Err(ocall::UNKNOWN) if left > 0 => {
                let _ = room::sleep(Duration::from_millis(RETRY_MS as u64));
                left = left.saturating_sub(RETRY_MS);
            }
            Err(code) => break (code, none),
        }
    };

    // 四、查到的那一枚（持树者经会话授进本域表里）：本域在表里认得出它吗（读数里的 `entry`）
    // ——它就是上面那一趟带回来的号。
    // 五、走完这一趟：说一句"我走了"（一字节帧，不带名字也不带入口）。板据此撤掉本域那一格、
    //     摘掉本域挂在板上的牌子，答一格 `OK`；本域不在板上那本账上则答 `UNKNOWN`。
    let bye = board::evict(talk, &link, Wait::AtMost(MS)).unwrap_or(BAD);

    say(&format!(
        "guest: reg={reg} find={find} entry={} bye={bye}",
        at.get()
    ));

    // 判据就地登记（用户裁定"服务台搬进 SUT"）：**只搬本域已经在判的东西**——那三样都是本站
    // 此刻就知道的期望（旧宿主靶上 `guest: reg=0 find=0` 那一行钉的就是它们）。
    {
        assert_eq!(reg, bcall::OK)
    }
    {
        assert_eq!(find, ocall::OK)
    }
    {
        {
            assert!(at != none);
        }
    }

    // 六、退场：一次往返，不留常驻（kernel 打的那一行就是这一格的读数）。
    let walked = reg == bcall::OK && find == ocall::OK && at != none;
    return Report::note(
        if walked { E_OK } else { E_TRIP },
        if walked {
            "guest: trip ok"
        } else {
            "guest: trip failed"
        },
    );
}

/// 哪里算不下去就报哪一句（kernel 收场时把这一句连同域号打出来）。
fn bail<'a>(note: &'a str) -> Report<'a> {
    return Report::note(E_TRIP, note);
}

/// 打一行。调试面是本域唯一的嘴（与 `echo` 用的是同一格）。
fn say(msg: &str) {
    let _ = debug::put(msg);
}
