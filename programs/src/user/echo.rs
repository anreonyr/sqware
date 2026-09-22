#![no_std]
#![no_main]

//! echo — **调试回显**：把**控制台服务**读到的一行原样写回去。
//!
//! 写的那一路仍走调试面（`DebugCall::Put`，class 8）——"域能说一句话"不依赖任何别的域；
//! **读的那一路换了来路**：从前是调试面的 `Get`（内核直通固件的 DBCN，固件代读串口 `RBR`），
//! 今天是从 `/device/uart` 那枚孔读——那台串口**只有一个读者**，而它的持有者是
//! `programs/src/driver/uart`：字节由它排空、由它交出来（见那边的头注）。
//!
//! ```text
//!   1  板那条路：seat(板) + claim(生我者, 板) + 另铸一枚问话孔给板（REGISTER 用）
//!   2  REGISTER "echo"：本域的入口经会话交给板（板据此看得见本域的死）
//!   3  树那条路：seat(树) + claim(生我者, 树) + 另铸一枚问话孔给持树者
//!   4  FIND "/device/uart"：**先找控制台**——找到就拿到"从设备读"的那一枚
//!   5  上树一趟：PART / LAND / FIND / take / TRIM（本域是第一位真客人）
//!   6  上树第二趟：**一串**——LIST 列根、NAME 按号翻名、LIST /device、NAME 一枚没铸过的号
//!   7  从控制台读：**一条消息 = 一次排空**（字节流，边界无意义）⇒ **攒够一行**写一行
//!   8  读到一行 `exit` 就退场（域退场 ⇒ 编排域收场 ⇒ 引导域退 ⇒ 停机）
//! ```
//!
//! # 为什么按**行**回显，而不是读到几个字节就写几个
//!
//! 写的那一侧是行语义（内核 `putln!`）：**一次 `put` 就是一行**。按字节回显会得到
//! `h\ne\nl\nl\no\n`——不是这台机器的怪癖，是"一次写必须是一条完整的字"这条红线在
//! 地板上的形态。读的那一侧是**字节流**（服务一次排空交一批），故本域攒够一行再写。
//!
//! # 为什么先找控制台、后落自己那块牌子
//!
//! 树上那两趟都是"查回来一枚"，而"认哪一枚"用的是**本端表里最后一份由持树者授进来的孔**
//! （`operator::take`）⇒ **先查的先认**。本域自己那块门牌也会被授回来一份，若先落牌子再找
//! 控制台，那一份就会把控制台那一枚盖过去（实测栽过：认错了孔，之后一个字节都读不到）。
//!
//! # 两条边界（设计红线）
//!
//! 1. **一台设备只有一个读者**：这条读口从前在内核的调试面手里（固件代读），今天在设备持有者
//!    手里——本域只是它的客人。调试面的读入（`DebugCall::Get`）留着，但**今天没有用家**。
//! 2. **一次写必须是一条完整的字**；写同一个设备的域不止一个时，混着写就会互相插字
//!    （旧树里 root 的设备直连写与服务的写就是这么插花的：`root: conssoll: coole nsole…`）。
//!
//! # 为什么它也上板（`board: true`）
//!
//! **本域要让人看得出它死了**。本域退场时开的那几枚孔随退出钩子封印 ⇒ 板当场看出"客人没了"
//! ⇒ 往死亡通知那条路推一格 ⇒ 装配者（编排域）据此记账并放下本域那个死域。本域是**最后一条**，
//! 故这一格同时就是"会话结束"的信号。挂不上板照旧回显——只是那条信号缺席。
//!
//! 非 UTF-8 的一行折成 `<non-utf8>` 再写（内核那一格要过 `str`，见 [`env::fid::DebugCall`] 的
//! 既有语义）。调试回显面对的是一台终端，够用——不为它动冻结面。

extern crate alloc;
extern crate programs;

// 共享物住在 supervisor 目录里，由各 bin 各自声明一次（见 `needs.rs` 头注）。
// 板与树：本域都只用**客侧**那几手。
use protocol::operator::client as operator;
use protocol::session::Quay;
use protocol::system::board::client as board;

use alloc::format;
use alloc::string::String;
use core::time::Duration;

use env::DBCN_MAX;
use env::{Name, PieToken, TaskId};
use protocol::operator::call as ocall;
use protocol::operator::{EntryId, Listing};
use protocol::system::board::call as bcall;
use runtime::env::debug;
use runtime::env::mail::{self, HolePie};
use runtime::env::room::{self, exit_with};
use runtime::env::unit as utask;

/// 本域挂在板上的名字（板按它分人；编排域表里那一条也叫这个）。
const ME: &str = "echo";

/// 要找的那位服务在树上的名字：**控制台**（`/device/uart`——名字用服务名）。
const WANT: &str = "uart";

/// 等板 / 等树 / 找一趟控制台的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 找不到就再问一次的间隔（毫秒）：门牌是驱动落的，本域可能比它先起。
const RETRY_MS: usize = 1;

/// 域自己的正常退场码（与 `programs::entry::EXIT_OK` 同号）。
const EXIT_OK: usize = 0;

/// 没搭上（找不到控制台）：一次往返都做不成 ⇒ 报这一格退场。
const E_NO_CONSOLE: usize = 1;

/// 一行的上界。更长的行**截断**回显（超过它的行不可能是 `exit`，故收场判据不受影响）；
/// 与设备侧那一条同值（`programs/src/driver/uart/main.rs::DRAIN_MAX` 那个层次的约定）。
const LINE_MAX: usize = 128;

/// 一次从控制台读多少字节的缓冲。
///
/// **必须 ≥ 对面一次排空的上界**（今天 `DRAIN_MAX` = 64）：孔那一格**装不下就答 `Denied`
/// 且一个字节都不动**（内核 `pull` 的口径，不是截断）⇒ 缓冲小了不是丢一行，是**读不动**。
/// 取 [`DBCN_MAX`]（256）留四倍余量。
const BUF_MAX: usize = DBCN_MAX;

const READY: &str = "echo: ready";
const NON_UTF8: &str = "<non-utf8>";

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    let _ = debug::put(READY);
    // 上板：**注册在回显之前**——板要能看见本域（见头注）。挂不上照旧回显。
    let reg = register();
    let _ = debug::put(&format!("echo: reg={reg}"));

    let Ok(sire) = utask::sire() else {
        exit_with(E_NO_CONSOLE)
    };
    // 树那条路：本域只开一条会话——先找控制台，再落自己那块牌子（次序见头注）。
    let Ok((tree, host)) = operator::open(sire, MS) else {
        exit_with(E_NO_CONSOLE)
    };
    let Ok(talk) = operator::ask_hole(host) else {
        exit_with(E_NO_CONSOLE)
    };

    // 一、**先找控制台**：`FIND /device/uart` ⇒ 那枚孔经会话授进本域表里。
    let console = find_console(&tree, talk, host);
    let _ = debug::put(&format!("echo: console={}", console.is_some()));

    // 二、上树一趟：**本域是第一位真客人**——把入口挂到树上、再查回来取一枚、剪掉一块空 Pane。
    let op = trip(&tree, talk, host);
    let _ = debug::put(&format!("echo: op={op}"));

    // 三、上树第二趟：**一串**（列号 → 按号翻名 → 列 `/device` → 问一枚没铸过的号）。
    let seq = serial(&tree, talk, host);
    let _ = debug::put(&format!("echo: seq={seq}"));

    let Some(console) = console else {
        exit_with(E_NO_CONSOLE)
    };
    let mut buf = [0u8; BUF_MAX];
    let mut line = [0u8; LINE_MAX];
    let mut n_line = 0usize;

    'echo: loop {
        // **一次读就是一批**（服务排空多少给多少，也可能是半行）：**阻塞读**，没有字节就在核里睡
        // ——不再轮询、不再空转（从前的 `IDLE_MS` 那一拍随调试面读入一起不用了）。
        // `Err` = 那枚孔封了（持设备的域没了）或装不下（见 [`BUF_MAX`]）⇒ 收场。
        let Ok(n) = console.pull(&mut buf) else { break };
        let Some(chunk) = buf.get(..n) else { break };

        for &b in chunk {
            match b {
                b'\n' | b'\r' => {
                    let text = &line[..n_line];
                    if text == &b"exit"[..] {
                        break 'echo;
                    }
                    let _ = debug::put(core::str::from_utf8(text).unwrap_or(NON_UTF8));
                    n_line = 0;
                }
                // 行太长：多的字节丢掉（截断回显），但行尾判据照旧。
                _ => {
                    if let Some(slot) = line.get_mut(n_line) {
                        *slot = b;
                        n_line += 1;
                    }
                }
            }
        }
    }

    exit_with(EXIT_OK)
}

/// 找控制台：`FIND /device/uart`，**找不到就再问**（有界）——门牌是驱动落的，本域可能比它先起。
///
/// 找到之后那一枚**从会话里**进本域表（报文里没有号，见 [`ocall`]）：认的是"持树者刚授进来的
/// 那一份"，而本域此刻**还没落自己的门牌** ⇒ 这一趟拿走的一定是它（次序见头注）。
fn find_console(link: &Quay, talk: PieToken, host: TaskId) -> Option<HolePie> {
    let (Ok(dir), Ok(want)) = (Name::new(protocol::driver::DIR), Name::new(WANT)) else {
        return None;
    };
    let path = [dir, want];
    let none = PieToken::NONE;
    let mut left = MS;
    let code = loop {
        let code =
            operator::ask(talk, link, host, ocall::FIND, &path, none, MS).unwrap_or(ocall::BAD);
        if code != ocall::UNKNOWN || left == 0 {
            break code;
        }
        let _ = room::sleep(Duration::from_millis(RETRY_MS as u64));
        left = left.saturating_sub(RETRY_MS);
    };
    if code != ocall::OK {
        return None;
    }
    Some(HolePie::from_token(operator::take(link, host)?))
}

/// 上板报到（与 `passer` 同一段前奏）：返板的答码（`bcall::OK` = 挂上了）。
fn register() -> u8 {
    let Ok(sire) = utask::sire() else {
        return bcall::BAD;
    };
    let Ok((link, board)) = board::open(sire, MS) else {
        return bcall::BAD;
    };
    let Ok(talk) = board::ask_hole(board) else {
        return bcall::BAD;
    };
    let Ok(entry) = mail::unseal_hole(board::ENTRY_MARK) else {
        return bcall::BAD;
    };
    let Ok(me) = Name::new(ME) else {
        return bcall::BAD;
    };
    board::ask(talk, &link, board, bcall::REGISTER, me, entry, MS).unwrap_or(bcall::BAD)
}

/// 上树一趟（装配单里本域 `operator: true`）：**分 → 落 → 寻 → 收 → 剪**五步。
///
/// 返最后那一格（剪的答码，`ocall::OK` = 五步都成）。中间任何一步不成 ⇒ 当场的答码就是
/// 返回值——**一格里已经有"死在哪一步"**，不需要另立读数。
///
/// 为什么这五步都要走：树上那四支判据各有各的门（分出第二层、落一枚真 Pie、寻回来把 Pie
/// 经会话授出、剪掉一块空 `Pane`），少走一步就有半条路从来没被走过。挂的是本域自己那一枚
/// 入口（与上板那一枚同一个记号），故它在树上是一枚普通 `Tile`，不是特权。
fn trip(link: &Quay, talk: PieToken, host: TaskId) -> u8 {
    let Ok(entry) = mail::unseal_hole(board::ENTRY_MARK) else {
        return ocall::BAD;
    };
    let Ok(name) = Name::new(ME) else {
        return ocall::BAD;
    };
    let path = [name];
    let none = PieToken::NONE;
    // 分出一块空 `Pane`、再在里面落一枚 `Tile`——**第二层**因此是实打实走出来的（不是构造出来的）。
    let a = operator::ask(talk, link, host, ocall::PART, &path, none, MS).unwrap_or(ocall::BAD);
    let b = operator::ask(talk, link, host, ocall::LAND, &path, entry, MS).unwrap_or(ocall::BAD);
    let c = operator::ask(talk, link, host, ocall::FIND, &path, none, MS).unwrap_or(ocall::BAD);
    // 寻回来的那一枚：**来源位是持树者**（号不从报文里走，故只能按"谁给的"认）。
    let got = operator::take(link, host).is_some();
    // **间接寻址那一手**：按同一条路问号，再拿号问名——两格都答得出，才说明这枚号是真坐标。
    // 这一格**下一步就被剪掉**，故号与名都得赶在 `trim` 之前问。
    let seek = operator::seek(talk, link, host, &path, MS);
    let pname = seek
        .ok()
        .and_then(|id| operator::name(talk, link, host, id, MS).ok());
    let (plate, pid) = match seek {
        Ok(id) => (ocall::OK, id.get()),
        Err(code) => (code, 0),
    };
    let d = operator::ask(talk, link, host, ocall::TRIM, &path, none, MS).unwrap_or(ocall::BAD);
    let _ = debug::put(&format!(
        "echo: tree part={a} land={b} find={c} got={got} trim={d} plate={plate} pid={pid} pname={}",
        pname.as_ref().map(|n| n.as_str()).unwrap_or("-"),
    ));
    d
}

/// 上树第二趟（**一串**）：列根 → 逐枚翻名 → 列 `/device` → 问一枚没铸过的号。
///
/// **名与号分开**那一刀的四格读数就落在这里：`list` 答号、`name` 按号答名（名字在答话那一侧，
/// 长短由那一帧说）。返这一趟的答码（`ocall::OK` = 全成）——每一格自己打一行，故中途断了也
/// 看得出断在哪一条。
fn serial(link: &Quay, talk: PieToken, host: TaskId) -> u8 {
    // 根那一层：**空路 = 根**。
    let Ok(root) = operator::list(talk, link, host, &[], MS) else {
        return ocall::UNKNOWN;
    };
    let _ = debug::put(&format!("echo: list root={}", ids_of(&root)));

    // 逐枚翻名：`0` 是真的第一个格子（**根没有号**），故它也该翻得出名字。
    let mut names = String::new();
    let mut code = ocall::OK;
    for id in root.iter() {
        if !names.is_empty() {
            names.push(',');
        }
        match operator::name(talk, link, host, id, MS) {
            Ok(name) => names.push_str(name.as_str()),
            Err(_) => {
                names.push('?');
                code = ocall::UNKNOWN;
            }
        }
    }
    let _ = debug::put(&format!("echo: list names={names}"));

    // 第二块 Pane：`/device`（那一段名字本域已经知道——头注里那两条来路之一）。
    let Ok(dir) = Name::new(protocol::driver::DIR) else {
        return ocall::UNKNOWN;
    };
    match operator::list(talk, link, host, &[dir], MS) {
        Ok(sub) => {
            let _ = debug::put(&format!("echo: list device={}", ids_of(&sub)));
        }
        Err(code) => {
            let _ = debug::put(&format!("echo: list device=err:{code}"));
            return ocall::UNKNOWN;
        }
    }

    // 一枚没铸过的号：**`UNKNOWN`，不是"答了一格空名字"**。
    let miss = operator::name(talk, link, host, EntryId::new(4095), MS).is_err();
    let _ = debug::put(&format!("echo: name miss={miss}"));
    if miss { code } else { ocall::BAD }
}

/// 一串号拼成 `0,3` 这样一段（读数用；一枚都没有拼成 `-`）。
fn ids_of(ids: &Listing) -> String {
    let mut out = String::new();
    for id in ids.iter() {
        if !out.is_empty() {
            out.push(',');
        }
        out.push_str(&format!("{}", id.get()));
    }
    if out.is_empty() {
        out.push('-');
    }
    out
}
