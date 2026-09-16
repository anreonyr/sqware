//! board — **公示板那两半**：板侧待客（[`serve`]），客侧问一句（[`call`]）。
//!
//! 两侧都用会话的**同一条**动作（[`Quay::pair`]）——"装上自己那一条、亮出名字、
//! 等对方把回话那一枚交进来"。两侧各装一条、各读一条，方向因此是天然分好的：
//!
//! ```text
//!   客侧                                    板侧
//!   pair("records")   ── 装我读的那条 ────▶ （板往它推答话）
//!   post(Query)       ── 我问 ────────────▶ pull_from ⇒ 一条问
//!                     ◀──────── 我答 ───── post(Reply)
//!   pull(Reply)       ◀─ 一句答
//!   pair("board")     ── 等它那条回来 ────▶ pair("board")  装它读的那条
//! ```
//!
//! **板不需要"客人 → 回信孔"的表**：回信走客人亲手交进来的那枚孔；而"这一问是谁问的"
//! 由内核在 Push 那一刻盖章（[`Pier::peer`]），不是报文里的字段。
//!
//! # 一线之差：入口的方向
//!
//! 挂上来（`REGISTER`）时，客人手里那枚入口要**经 `Accord` 交出去**——板据此认得
//! "谁挂的"（[`bcall::hang_in`] 返的正是**种在板表里**的那个号，客人把它写进帧）。
//! 查到（`LOOKUP`）时反过来：板把自己那一份**转授**给客人（[`bcall::give`]），
//! 那一枚经会话进客人的表（[`take`]）——**两个方向都不靠报文里的裸号**。
//!
//! # 字节长什么样
//!
//! 帧形、三个动作码、一处上界全在 [`protocol::board::call`]；本文件只做
//! "读一条 → 交给板 → 回一句"，一个字节都不自己编。

use alloc::vec::Vec;

use env::{Name, PieToken, TaskId};
// 三个动作码、帧形、一处上界都归协议层；本层只转发一次，不自己编字节。
pub use protocol::board::call::{ASK, LOOKUP, REGISTER, UNREGISTER};

/// 转发层的别名：本模块自己也有一个 `call`（客侧那一句），故给协议那层换个短名。
use protocol::board::call as bcall;
use protocol::board::{Board, Fail, Free, Probe};
use protocol::session::{Claim, Pier, Quay};
use runtime::core::unit::{self, Join};

/// 板侧装的那一条：**客人的问从它进来**。
///
/// 名字要与宿主自己的通道错开（宿主可能已经用 `records` 跟它的父域建了一条）——
/// 板这一对名字只在本域内用，故叫得再直白也无妨。
const ASK_IN: &str = "board-ask";
/// 客侧装的那一条：**答话从它回去**。两侧各装一条、各读一条。
const SAY_BACK: &str = "board-say";

/// 一次等客/等答的上限（毫秒）。**必须有界**——对面死在头几步时这边不能陪着挂死。
pub const WAIT_MS: usize = 1000;

/// 答话的码——与 [`Fail`] 一一对应，另加"读不懂这一问"。
pub const OK: u8 = 0;
pub const UNKNOWN: u8 = 1;
pub const TAKEN: u8 = 2;
pub const DENIED: u8 = 3;
pub const FULL: u8 = 4;
pub const BAD: u8 = 5;

// ── 板侧 ────────────────────────────────────────────────────

/// 起一枚板线程。返它的 task id —— 客人拿它开门（[`call`] 的 `holder`）。
///
/// 板是**宿主域里的一枚线程**，不是域：故 `store`（跟谁建第一条路）由宿主告诉它。
/// 板**不返回**（长期待客），故宿主 `mem::forget` 那个 `Join`——不等它的结果。
pub fn start(store: TaskId, probe: Probe, free: Free) -> TaskId {
    let node: Join<()> = unit::closure(move || serve(store, probe, free));
    let id = node.id();
    core::mem::forget(node);
    id
}

/// 长期待客：**一次只接一位客**（这一版不做并发，一问一答）。
///
/// 客人退场后它那两枚孔自然失效，板上那一行由核心的惰性剔除扫掉——这里不管清理。
fn serve(store: TaskId, probe: Probe, free: Free) {
    let mut board = Board::new(probe, free);
    let (ask_in, say_back) = match (Name::new(ASK_IN), Name::new(SAY_BACK)) {
        (Ok(a), Ok(b)) => (a, b),
        _ => return,
    };
    loop {
        let Some(quay) = wait(store, ask_in, say_back) else {
            return;
        };
        let Some(client) = quay.find_pier(say_back) else {
            continue;
        };
        let Some(want) = read(&client) else { continue };
        let bytes = hand(&mut board, &want, client.peer());
        let _ = client.post(&bytes);
    }
}

/// 板侧第一步：订一条路（读 `board`、写 `records`），返整座码头。
fn wait(store: TaskId, ask_in: Name, say_back: Name) -> Option<Quay> {
    let mut quay = Quay::open(store);
    quay.pair(ask_in, WAIT_MS).ok()?;
    quay.pair(say_back, WAIT_MS).ok()?;
    let pier = quay.find_pier(ask_in)?;
    // **把名字牌读走**：`pair` 亮名字那一步往这枚孔里推了一张 40 字节的牌，而孔是
    // **单槽**——不读走，客人随后推来的那一句 Query 就只会在槽外等（`push` 满则让位）。
    // 牌的内容（名字 + 号）在归位那一刻就用完了，故这里读掉即丢。
    let _ = pier.drain();
    Some(quay)
}

/// 板侧第二步：读一条。**上界就是帧的上界**（[`ASK`]）——本协议只有一种消息，
/// 故不必先问长度：一张够大的缓冲读一次，多出来的那部分永远是空的。
fn read(pier: &Pier) -> Option<Vec<u8>> {
    let mut buf = [0u8; ASK];
    let n = pier.pull(&mut buf, WAIT_MS).ok()?;
    Some(buf.get(..n)?.to_vec())
}

/// 板侧第三步：把一条问交给板，编出一句答。
fn hand(board: &mut Board, want: &[u8], who: TaskId) -> Vec<u8> {
    let (op, name) = match decode(want) {
        Some(pair) => pair,
        // 读不懂就答 `BAD`——不猜、不崩。
        None => return alloc::vec![BAD],
    };
    let answer = match op {
        REGISTER => match entry_of(want) {
            Some(entry) => board.register(name, entry, who).map(|_| OK),
            None => Err(Fail::Denied),
        },
        UNREGISTER => board.unregister(name, who).map(|()| OK),
        LOOKUP => board
            .lookup_after(name, |entry| {
                // 查到就**转授一份**给客人：入口不从报文里走，从会话里走。
                let _ = bcall::give(entry, who);
            })
            .map(|_| OK),
        // 没见过的动作码：与"这个名字不在板上"同一句话（不另立一格）。
        _ => Err(Fail::Unknown),
    };
    alloc::vec![code(answer.err())]
}

/// 失败域 → 答话的码。
fn code(fail: Option<Fail>) -> u8 {
    match fail {
        None => OK,
        Some(Fail::Unknown) => UNKNOWN,
        Some(Fail::Taken) => TAKEN,
        Some(Fail::Denied) => DENIED,
        Some(Fail::Full) => FULL,
    }
}

// ── 客侧 ────────────────────────────────────────────────────

/// 客侧：问一句、取一句答。返 `(答话的码, 这座码头)`。
///
/// 返 [`OK`] 只说明"话问到了"：**查到的那枚入口由会话交进本端表里**（板用 `Accord`
/// 授出），收它的是随后的 [`take`]——码头交回给调用方就是为这一步。
pub fn call(
    holder: TaskId,
    op: u8,
    name: Name,
    entry: PieToken,
    ms: usize,
) -> Result<(u8, Quay), Fail> {
    let mut quay = Quay::open(holder);
    let say_back = Name::new(SAY_BACK).ok().ok_or(Fail::Unknown)?;
    let ask_in = Name::new(ASK_IN).ok().ok_or(Fail::Unknown)?;
    match quay.pair(say_back, ms) {
        Ok(_) => {}
        Err(claim) => return Err(map_claim(claim)),
    }
    // 挂上去：**入口经会话交给板**（`Accord` 一份），它返"种在板表里"的那个号——
    // 那个号才是板认得的坐标，故写进帧里。
    let at = match op {
        REGISTER => Some(bcall::hang_in(entry, holder).map_err(|()| Fail::Denied)?),
        _ => None,
    };
    let frame = bcall::pack(op, name, at);
    let pier = quay.find_pier(say_back).ok_or(Fail::Unknown)?;
    pier.post(&frame).map_err(|()| Fail::Unknown)?;
    let mut reply = [0u8; 1];
    match pier.pull(&mut reply, ms) {
        Ok(1) => {}
        _ => return Err(Fail::Unknown),
    }
    // 查到的那一枚：板上把它推给了我们，认领归位（板装的那一条名字就叫 `ask_in`）。
    if op == LOOKUP {
        quay.pair(ask_in, ms).map_err(map_claim)?;
    }
    Ok((reply[0], quay))
}

/// 客侧：查到的那枚入口在**本端表里**的句柄（板经 `Accord` 授出、会话交进来的）。
///
/// 这一枚就是"往那个服务说话"的句柄——**它不是报文里的号**，故不存在"两边编号
/// 对不上"这一整类问题。
pub fn take(quay: &Quay) -> Option<PieToken> {
    let ask_in = Name::new(ASK_IN).ok()?;
    let pier = quay.find_pier(ask_in)?;
    Some(pier.at_peer())
}

fn map_claim(claim: Claim) -> Fail {
    match claim {
        Claim::Nameless => Fail::Denied,
        Claim::NoPeer => Fail::Unknown,
        Claim::Timeout => Fail::Unknown,
        Claim::Partial => Fail::Full,
    }
}

// ── 帧 ──────────────────────────────────────────────────────

/// 解码一条问：`[op][name][入口号 8]`。**读不懂就答 `BAD`**，不猜。
fn decode(bytes: &[u8]) -> Option<(u8, Name)> {
    let op = *bytes.first()?;
    let name = bcall::name_of(bytes.get(1..)?)?;
    Some((op, name))
}

/// 帧里那一格入口号：客人在板表里的那一枚（客人挂上来时 `hang_in` 换回来的）。
///
/// **只有 `REGISTER` 有**——它是"你挂上来的那一份，我这边叫什么"，不是"你的入口是几号"
/// （两个编号空间不同源，互相拿错正是旧树 `[33..41]` 那一格的病）。
fn entry_of(bytes: &[u8]) -> Option<PieToken> {
    let at = bytes.get(1 + env::wire::NAME_LEN..ASK)?;
    Some(PieToken::new(
        u64::from_le_bytes(at.try_into().ok()?) as usize
    ))
}
