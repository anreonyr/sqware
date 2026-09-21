//! coalition::server — **结盟服务那一台**：一枚线程守着盟册（一条关系 + 一枚计数器）。
//!
//! 载体是 rtc / principal 那一面已经量过的形状——**门牌自带回信孔**：客人替这一趟铸一枚回信
//! 孔借过来、把帧推上门牌，本域从**门牌那一枚**读（发送者由内核在 `Push` 那一刻盖章），办完事
//! 从那枚孔答回去、当场放下。故这里**没有客人账**：一位客人不需要本域记住任何东西。
//!
//! ```text
//!   起手：读自己的 Sire（**只为上板与上树两条会话**——盟无主，之后不落任何字段）
//!         → 上板（板看得见本域的死）→ 铸门牌那一枚
//!           经 LAND 落到树上 `/sys/coalition`，再 FIND 回来验一遍
//!         → FIND `/sys/principal`（**带重试**）拿一份身份服务的门牌
//!   常驻：一只组等门牌那一枚 —— 读一帧（连发送者）→ 先过名册问"你是谁" → 交给核心 → 答回去
//! ```
//!
//! **两条锚为什么都在这儿**：`Sire` 是内核盖的（比任何自报都硬，且是弱引用：装配者一退它
//! 就答 0），树那条路是"按名找人"的现成一步；而身份那一份门牌**只能按名字找**——本域不是
//! 装配者，拿不到它手里那一份副本（正文 K7 的被否项：转授要新装配机制）。

use alloc::format;
use core::time::Duration;

use env::{HoleDir, Name, PieToken, TaskId};
use protocol::coalition::call as ccall;
use protocol::coalition::core::{Coalition, CoalitionId, Fail};
use protocol::operator::call as ocall;
use protocol::operator::client as operator;
use protocol::principal::call as pcall;
use protocol::principal::client::Face;
use protocol::principal::core::PolicyId;
use protocol::session::Quay;
use protocol::session::call as scall;
use protocol::system::board::call as bcall;
use protocol::system::board::client as board;
use runtime::core::tole::Tole;
use runtime::env::mail::{self, HolePie};
use runtime::env::room::{self, exit_with};
use runtime::env::unit as utask;

/// 等板 / 等树 / 问名册的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 身份服务那份门牌可能落得比本域晚：找不到就再问一次的间隔（毫秒）。
const RETRY_MS: usize = 1;

/// 起不来时的编号（指死在头几步的哪一步）。
///
/// **照实记：本域比 principal 那一台少一个编号**（它那儿有一格 `E_BOOK`）——空册起手**不失败**
/// （`Coalition::new` 不分配），故没有"起那本账失败"这一步可指。
const E_SIRE: usize = 1;
const E_BOARD: usize = 2;
const E_TREE: usize = 3;
const E_DESK: usize = 4;
const E_FACE: usize = 5;

/// 三格答码共用的"没走到 / 读不懂"那一格（与树自己的 [`ocall::BAD`] 同值）。
const BAD: u8 = ocall::BAD;

/// 起服务：**读锚 → 上板 → 铸门牌上树 → 找身份那一份门牌 → 一枚线程招待所有客人**。
pub fn serve() -> ! {
    // 一、锚：`Sire` = 装配者。**只为上板与上树两条会话**——盟无主，核心不需要它
    //     （对照 principal：那边把它当名册钥匙，注入核心那一格）。
    let Ok(assembler) = utask::sire() else {
        say("coalition: no sire");
        exit_with(E_SIRE);
    };

    // 二、上板：只为让板看得见本域的死（它常驻，编排域据此记账）。
    let Ok((_link, board_link)) = board::open(assembler, MS) else {
        say("coalition: no board");
        exit_with(E_BOARD);
    };
    if board::ask_hole(board_link).is_err() {
        say("coalition: no board ask");
        exit_with(E_BOARD);
    }

    // 三、门牌那一枚：本域自己开（`entry` 是服务入口的通用记号）。
    let Ok(entry) = mail::unseal_hole(bcall::ENTRY_MARK) else {
        say("coalition: no entry");
        exit_with(E_TREE);
    };

    // 四、上树：分 `/sys`、落 `/sys/coalition`、再查回来验一遍（同 router / rtc / principal）。
    let Ok((tree, host)) = operator::open(assembler, MS) else {
        say("coalition: no tree link");
        exit_with(E_TREE);
    };
    let Ok(talk) = operator::ask_hole(host) else {
        say("coalition: no tree ask");
        exit_with(E_TREE);
    };
    serve_tree(&tree, talk, host, entry);

    // 五、**身份那一份门牌**：本域是它的客人（K7）。带重试——它可能落得比本域晚。
    let Some(face_entry) = find_face(&tree, talk, host) else {
        say("coalition: no identity face");
        exit_with(E_FACE);
    };
    let Ok(face) = Face::of(face_entry) else {
        say("coalition: bad identity face");
        exit_with(E_FACE);
    };

    // 六、一本空册：一枚号都还没铸（**起手不失败**——空册不分配）。
    let mut book = Coalition::new();

    // 七、常驻：**一只组等门牌那一枚**。这是常态，故等待没有期限。
    let Ok(tole) = Tole::unseal(false) else {
        say("coalition: no group");
        exit_with(E_DESK);
    };
    let entry_hole = HolePie::from_token(entry);
    if tole.attach(&entry_hole, HoleDir::Pull).is_err() {
        say("coalition: entry not hung");
        exit_with(E_DESK);
    }

    // 一问的上界就是 `ASK_LEN`（`pack_ask` 产出的就是这个长度）。
    let mut buf = [0u8; ccall::ASK_LEN];
    loop {
        if tole.await_(usize::MAX).is_err() {
            exit_with(E_DESK);
        }
        // 门牌是**单槽**：一次醒来的这一批要取干净（可能不止一位客人）。
        while let Ok((len, from)) = entry_hole.pull_timeout_from(&mut buf, 0) {
            turn(&mut book, &face, from, &buf[..len]);
        }
    }
}

/// 门上一句话：解帧 → 先过名册 → 交给核心 → **从这一趟自带的那枚孔答回去**。
///
/// 认那枚孔靠 [`scall::find`] 的两格正判据（谁给的 + 记号）；`from` 是**内核盖的发送者**。
fn turn(book: &mut Coalition, face: &Face, from: TaskId, frame: &[u8]) {
    let Some((op, a, b)) = ccall::unpack_ask(frame) else {
        // 不是那个形状：不猜、不动账、也不回话——没有可信的"往哪回"。
        return;
    };
    let Some(back) = scall::find(from, ccall::BACK) else {
        // 这一趟没把回信孔交进来（或交得不成）：没有可回的路，账一动不动。
        return;
    };
    let _ = HolePie::from_token(back).push(&answer(book, face, from, op, a, b));
    let _ = mail::release(back);
}

/// 把一句问交给核心，编出一句答（**一格**：读不懂也答，答 `BAD`）。
///
/// **先读动作码、再解载荷**；三条**写**原语同一个起手：**先拿发送者过名册**（[`who`]）。
/// 两条读不过名册——`amid` 的 `p` 是问的人给的标签（K6）。
fn answer(
    book: &mut Coalition,
    face: &Face,
    from: TaskId,
    op: u8,
    a: u64,
    b: u64,
) -> [u8; ccall::REPLY_LEN] {
    match op {
        // `found` 的钥匙是"你得是个已绑定的身份"（K3），**解析出来的那条号只当门卫**：
        // 盟无主（K2），不记铸造者——全族唯一一处。
        ccall::FOUND => match who(face, from) {
            Ok(_) => ccall::reply_value(book.found()),
            Err(fail) => status(fail),
        },
        ccall::ENTER => match who(face, from) {
            Ok(w) => match book.enter(w, CoalitionId::new(a as usize)) {
                Ok(()) => ccall::reply_status(ccall::OK),
                Err(fail) => status(fail),
            },
            Err(fail) => status(fail),
        },
        ccall::LEAVE => match who(face, from) {
            Ok(w) => match book.leave(w, CoalitionId::new(a as usize)) {
                Ok(()) => ccall::reply_status(ccall::OK),
                Err(fail) => status(fail),
            },
            Err(fail) => status(fail),
        },
        ccall::AMID => {
            match book.amid(PolicyId::new(a as usize), CoalitionId::new(b as usize)) {
                // "不在"是一句答（`Ok(false)`），"查无此盟"才是这一格。
                Ok(yes) => ccall::reply_yes(yes),
                Err(fail) => status(fail),
            }
        }
        // 没见过的动作码：与"这一问读不懂"同一格（不另立一格）。
        _ => ccall::reply_status(ccall::BAD),
    }
}

/// 发送者此刻代表谁——**"self"的全部护栏就是这一句**（正文"已知边界"）。
///
/// **照实记：两条失败压成一格**——"这条 TID 没绑"与"身份服务答不上来（超时 / 对面没了）"。
/// 压它的理由同 rtc 那一格：**调用方的下一步在两种情况下相同**（别指望这条路）；principal
/// 那枚 `Denied` 翻不过来，因为本族的 `Denied` 是空的（盟无主）。
fn who(face: &Face, from: TaskId) -> Result<PolicyId, Fail> {
    face.resolve(from, MS)
        .map_err(|_| Fail::Unknown)?
        .ok_or(Fail::Unknown)
}

/// 失败域 → 答话那一格（三格答码只此一处编）。
fn status(fail: Fail) -> [u8; ccall::REPLY_LEN] {
    ccall::reply_status(ccall::fail_to_code(Some(fail)))
}

/// 找**身份服务**那份门牌：`FIND "/sys/principal"`，**找不到就再问**（有界）。
///
/// 门牌是 principal 自己跑完它那一段才落下的（它比本域先起来，但"就绪"与"上树"不是同一步）
/// ——故这一趟**必须有重试**：撞 `UNKNOWN` 就睡 `RETRY_MS` 再来，总预算 [`MS`]。
///
/// 取回的那一枚按"谁给的"认（[`operator::take`] 取满足条件的**最后**一枚）：本域表里此刻
/// 还有刚验完的那一枚自己的门牌副本，故**最后那一枚**正是这一趟找回来的。
fn find_face(link: &Quay, talk: PieToken, host: TaskId) -> Option<PieToken> {
    let (Ok(dir), Ok(name)) = (Name::new(pcall::DIR), Name::new(pcall::NAME)) else {
        return None;
    };
    let path = [dir, name];
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
    operator::take(link, host)
}

/// 上树那一趟：**分目录 → 落门牌 → 查回来验一遍**（同 rtc / principal 那一趟）。
///
/// `got` 只是"认出了那一枚"；它指不指得回原物，由**真客人**（`user/member`）证——它照同一条路
/// 找上门、立一枚盟、进进出出。故本域不自问自答。
fn serve_tree(link: &Quay, talk: PieToken, host: TaskId, entry: PieToken) {
    let (Ok(dir), Ok(me)) = (Name::new(ccall::DIR), Name::new(ccall::NAME)) else {
        say("coalition: tree: bad name");
        return;
    };
    let path = [dir, me];
    let none = PieToken::NONE;
    let part = operator::ask(talk, link, host, ocall::PART, &[dir], none, MS).unwrap_or(BAD);
    let land = operator::ask(talk, link, host, ocall::LAND, &path, entry, MS).unwrap_or(BAD);
    let find = operator::ask(talk, link, host, ocall::FIND, &path, none, MS).unwrap_or(BAD);
    let got = operator::take(link, host).is_some();
    say(&format!(
        "coalition: tree part={part} land={land} find={find} got={got} entry={}",
        entry.get()
    ));
}

/// 打一行。调试面是"服务还没起来的嘴"：本域没有控制台，只有它。
fn say(msg: &str) {
    let _ = runtime::env::debug::put(msg);
}
