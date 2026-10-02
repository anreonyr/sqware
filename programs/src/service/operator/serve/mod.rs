//! 自己的域里的一枚线程守着那棵树（一枚线程 + 一个组，无轮询）。
//! | 收的是什么 | 交给谁 |
//! |---|---|
//! | 提示之路上的一条路（ocall::TipIn::Plate） | self::plate::plate：前缀立窗格 ＋ 末段落格 |
//! | 提示之路上的一位客人（ocall::TipIn::Guest） | 客人账（Desk::admit） |
//! | 客人的一句问（ocall::Req） | self::answer::answer：八条原语 |
//! | 订阅关系（`watch` 那一面 ＋ 每次真改了树） | 订阅册（self::watch::Watchers）：事件写进订阅者那一页 |
//! **为什么就一枚线程**：树上那几枚是**一枚只在持它的那张表里有意义的句柄**（`PieToken` = "我这张
//! 表里的第几个"）。"查到了要把 Pie 授出去"必须由**持有那一枚的那张表**来做——故所有条目只能住
//! 同一张表，也就是同一枚线程。板那一台栽过这条（每位客人一枚待客线程 ⇒ 甲的条目在甲的表里，
//! 乙来查时判它"已死"、也授不出去，症状是"刚挂上的名字，别人一查就是 `Unknown`"）。
//! **三侧分家**：两侧共用的图与说明见 super 的"载体"那一节，帧与记号见
//! :operator

use alloc::vec::Vec;
use env::{HoleDir, Mark, PieToken, TaskId, Wait};
use runtime::PAGE_SIZE;
use runtime::core::res::pile::Pile;
use runtime::core::res::port::{self, Access, Policy};
use runtime::env::mail;

use protocol::communication::hand::Sender;
use protocol::debug;
use protocol::service::operator as ocall;
use protocol::service::operator::Grant;
use protocol::service::operator::grant::grant_of;

use crate::service::operator::core::Operator;
use crate::system::common::face::desk::{Desk, DeskFail, Guest};
use crate::system::common::life::service::Start;
use crate::unit::operator::E_TREE;

use self::answer::answer;
use self::plate::plate;
use crate::service::operator::claim::{ask_of, mark_of, reply_of};

mod answer;
mod door;
mod plate;
mod watch;

/// 还在"补齐两本账"（答话路未认领 / 问话孔未挂上）时，一轮等多久（毫秒）
/// **不是轮询**：账补齐之后这一等就变成 Wait::Forever（由组唤醒）；这个短期限只在装配窗口
/// 里用——那几步的到达是**别人**在做（装配者转授、客人自己交孔）
const SETTLE_MS: usize = 1;

/// **一位"提示到了、答话路还没在本表里"的客人**（一位一格）。
///
/// 契约是"提示在转授之后"（见 `bridge::attach` 那一节：先 `hand` 把答话路交给持树者、
/// 再推这一条提示），而**量到过这条契约被破**：全控制台 194 份里 3 份落在这一格上，
/// 且与 `mail: hand stuck` **一一对应**（同样那 3 份、无一例外）。
/// 从前这一格**就地丢掉**，代价是那位客人**永远进不了这本账**：它的问话孔没人挂进组，
/// 它此后每一问都压在孔上（实测压到 16 s，直到收场）——而丢的那一下，
/// 客人那一侧看到的是"我在等一个永远不会来的回话"。
struct Late {
    /// 哪位客人
    who: TaskId,
    /// 第一次没认到的那一刻（`chrono::clock()`，纳秒）——额度按**毫秒**算，不按轮数：
    /// 本函数每轮都跑，而有客人问话时 `pile.await_` 可以立刻返回，轮与轮之间并不等长。
    since: u64,
}

/// **等答话路浮出来**的额度（毫秒）。它是一条**已经发出去**的转授（`port::ship`）——
/// 不是"还没做"，故这一格不设大：到点仍没有就不是时序问题，而是该往上一路查（那条读数会
/// 把号与等了多久一起报出来）。
const LATE_MS: usize = 1_000;

/// **本族认得的全部问话孔记号**：控制面那一枚 ＋ 操作面**每一位**各一枚（`Grant::ALL` 的位数）
/// **一处给**：Desk::arm_pending 逐枚试、ask_of 逐枚比——两处读的都只有这一个数组。
/// 加一位 `Grant` 就要在这里加一枚（数组长度写死是 `const` 的代价：值与 `Grant::ALL` 对齐
/// 由 `count_under` 那一台探针在树上量——它数的就是这一族有几格）。
const MARKS: [Mark; 9] = [
    ocall::ASK_MARK,
    Grant::Part.mark(),
    Grant::Land.mark(),
    Grant::Find.mark(),
    Grant::Trim.mark(),
    Grant::List.mark(),
    Grant::Seek.mark(),
    Grant::Name.mark(),
    Grant::Watch.mark(),
];

/// 起服务：**上板 → 铸提示孔交给装配者 → 一枚线程招待所有客人**
/// settle）：它是"装配侧 → 持树者"的唯一一条路，故**不必另开一条到自己的会话**
/// "答话路必已在本表里"（转授在前、提示在后）
pub fn serve() -> Result<(), Start> {
    // （从 `args` 里掏一格那条绕路已退：它存在只因为 iii 让三枚与编排者同域）。
    let assembler = runtime::env::unit::sire();
    let Ok(tip) = mail::unseal_hole(ocall::TIP_MARK) else {
        return Err(Start::Tree(E_TREE));
    };
    let tip_hole = mail::HolePie::from_token(tip);
    if port::ship(
        &tip_hole,
        assembler,
        Access::FETCH | Access::STORE,
        Policy::VEST,
    )
    .is_err()
    {
        return Err(Start::Tree(E_TREE));
    }
    // **一个组**：提示孔 + 每位客人的问话孔。提示孔也挂进来，故"来客人了"与"有人问话"
    // 是**同一个等待**。本线程独享它（`shared = false`）。
    let Ok(pile) = Pile::unseal(false) else {
        return Err(Start::Desk(E_TREE));
    };
    if pile.attach(&tip_hole, HoleDir::Pull).is_err() {
        return Err(Start::Desk(E_TREE));
    }

    // 必须说得出这一句（与三台驱动、设备账那两处**同一手**，见 `programs/src/unit/catalog.rs` 那一格）。
    let _ = protocol::communication::session::establish::endpoint(
        runtime::env::unit::sire(),
        env::Mark::of(crate::unit::READY),
        env::Wait::POLL,
    );

    let mut tree = Operator::new();
    // **持树者自己那一具架**（事件住它那一页里）：开不出来这一台就起不来（与"桌"同一条处置）。
    let mut watchers = match watch::Watchers::open() {
        Ok(watchers) => watchers,
        Err(()) => return Err(Start::Desk(E_TREE)),
    };
    let mut desk = Desk::new();
    let mut query: Option<protocol::service::identity::client::TaskQuery> = None;
    // **提示到了、答话路还没认到的那几位**（见 `Late`）：册子小、异常才非空。
    let mut late: Vec<Late> = Vec::new();
    let mut buf: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    if buf.try_reserve_exact(PAGE_SIZE).is_err() {
        return Err(Start::Room(E_TREE));
    }
    buf.resize(PAGE_SIZE, 0);
    let mut outs: Vec<Outbox> = Vec::new();
    let mut settle_rounds: usize = 0;
    // **招待活动的读数**（封顶 40 行 ＋ 每 500 行留一行）：醒来这一次，是"认得的客人"还是
    let mut wakes: usize = 0;
    // **风暴的形状**：连着几次是"同一枚孔 ＋ 同一个动作码"。
    let mut last_tok = PieToken::NONE;
    let mut last_code: u8 = 0xff;
    let mut streak: usize = 0;
    loop {
        if query.as_ref().is_some_and(|bundle| !bundle.available())
        {
            query = None;
            debug::put("operator: retired unavailable identity query bundle");
        }
        // 一、补齐那几件事（收提示之路上那三种帧；认领答话路；认出问话孔并挂组）。
        let settling = settle(&mut desk, &pile, &tip_hole, &mut query, &mut tree, &mut watchers, &mut late);
        settle_rounds = if settling {
            settle_rounds.saturating_add(1)
        } else {
            0
        };
        if settling && settle_rounds % 2000 == 0 {
            unarmed_report(&desk, settle_rounds);
        }
        // 二、等一格有事。**一个等待**：提示孔或任意一位客人的问话孔。
        let millis = if settling {
            Wait::AtMost(SETTLE_MS)
        } else {
            Wait::Forever
        };
        let Ok(Some((tok, dir))) = pile.await_(millis) else {
            let _ = desk.sweep();
            continue;
        };
        wakes = wakes.saturating_add(1);
        // `serve_one` 返的是**这一问的动作码**（首格，`None` = 什么也没读到）——"读到没有"与
        // "要的是什么"是同一趟的两个事实，故一处交出来（见它那一节）。
        let code = if tok != tip
            && let Some(guest) = desk.guest(tok).copied()
        {
            serve_one(&mut tree, &mut watchers, guest, query.as_ref(), &mut buf, &mut outs)
        } else {
            // 但**别的号**就是在查的那一形——组里挂着一枚"一直报就绪、却没人招待"的孔。
            if tok != tip {
                debug!(
                    "operator: wake unknown tok={} push={}",
                    tok.get(),
                    matches!(dir, HoleDir::Push)
                );
            }
            None
        };
        let read = code.is_some();
        let code = code.unwrap_or(0xff);
        if tok == last_tok && code == last_code {
            streak = streak.saturating_add(1);
        } else {
            streak = 1;
            last_tok = tok;
            last_code = code;
        }
        if wakes <= 40 || wakes % 500 == 0 {
            let mut unarmed = 0usize;
            desk.unarmed_each(|_| unarmed += 1);
            debug!(
                "operator: woke n={} tok={} tip={} known={} read={} code={} streak={} guests={} unarmed={}",
                wakes,
                tok.get(),
                tok == tip,
                tok == tip || desk.guest(tok).is_some(),
                read,
                code,
                streak,
                desk.occupied(),
                unarmed,
            );
        }
        // 三、**看出来的**那一档：那一枚答不出 ⇒ 剔格子（没有"他说走了"那一档）。
        // **读者结清（B-a）**：剔的同时把它挂进组的那一枚问话孔**摘掉**——不摘就是一格再也醒不
        let _ = desk.sweep_each(|gone| {
            if let Some(ask) = gone.ask {
                let _ = pile.detach(&mail::HolePie::from_token(ask), HoleDir::Pull);
            }
            // **答话那一格也一起办**：先非阻塞地问一句"上一答被取走了没有"（Sender::settle）
            if let Some(at) = outs.iter().position(|o| o.who == gone.who)
                && outs[at].send.settle()
            {
                let _ = outs.swap_remove(at);
            }
        });
    }
}

fn valid_tip_back(back: PieToken, from: TaskId) -> bool {
    matches!(mail::reserve(back), Ok((vestor, owner, mark))
        if vestor == from && owner == from && mark == ocall::TIP_BACK)
}

fn tip_ack(back: PieToken, status: u8, id: ocall::EntryId) {
    let hole = mail::HolePie::from_token(back);
    let mut bytes = [0u8; 9];
    bytes[0] = status;
    bytes[1..].copy_from_slice(&(id.get() as u64).to_le_bytes());
    let _ = hole.push(&bytes, Wait::AtMost(1000));
    let _ = mail::release(back);
}

/// 补齐那几件事，返"还有没有没补齐的"
/// - **提示之路**：装配者推来的三形，**首格 `kind` 分派**（见 ocall::TipIn）
/// （plate），不经会话、不当自己的客人
/// - **一位客人**（ocall::TipIn::Guest）：`admit` 收进来；答话路还没在本表里 ⇒ **留在册上**
///   逐轮重问（见 [`Late`]）——契约说"提示在转授之后"，而破约时**丢**的代价是那位客人
///   永远进不了这本账
/// - 门禁接线：装配者给出完整 authority 与三枚入口；整束验证失败即清空。
fn settle(
    desk: &mut Desk,
    pile: &Pile,
    tip: &mail::HolePie,
    query: &mut Option<protocol::service::identity::client::TaskQuery>,
    tree: &mut Operator,
    watchers: &mut watch::Watchers,
    late: &mut Vec<Late>,
) -> bool {
    // 提示：拉干净（单槽，一位客人一条）。**非阻塞**——它的到达是别人在做的事。
    // 缓冲按**最长那一形**备（立一条路），故另两形也吃得下——小缓冲会把长帧读成"读不懂"。
    let mut frame = [0u8; ocall::TIP_LEN];
    let mut pending = false;
    let now = || runtime::env::chrono::clock();
    loop {
        let Ok((n, from)) = tip.pull(&mut frame, Wait::POLL) else {
            break;
        };
        // 提示通道是 Control 的特权装配边；持孔不等于有权替换身份权威。
        if from != runtime::env::unit::sire() {
            debug::put("operator: foreign bootstrap tip");
            continue;
        }
        // **首格 `kind` 决定形状**：表外的 kind / 长度不对 ⇒ 读不懂。这条路上没有答话那一格，
        // 故只能**报一句**（把那一格 kind 一起报出来，"读不懂的是哪一形"要看得见）。
        let Some(rec) = ocall::TipIn::fetch(&frame[..n]) else {
            debug!("operator: tip unreadable (kind={})", frame[0]);
            continue;
        };
        match rec {
            ocall::TipIn::Plate { road, leaf, permit, owner, replace, back } => {
                if !valid_tip_back(back, from) {
                    continue;
                }
                match plate(tree, &road, leaf, permit, owner, replace) {
                    Ok((id, changes)) => {
                        for change in &changes { answer::changed(tree, watchers, change); }
                        tip_ack(back, ocall::OK, id);
                    }
                    Err(fail) => tip_ack(back, ocall::fail_to_code(Some(fail)), ocall::EntryId::new(0)),
                }
            }
            ocall::TipIn::Abort { road, leaf, back } => {
                if !valid_tip_back(back, from) { continue; }
                let result = match tree.seek(&road) {
                    Ok(id) if tree.reference(id) == Some(leaf) => tree.trim(id),
                    _ => Ok(None),
                };
                if let Ok(Some(change)) = &result { answer::changed(tree, watchers, change); }
                tip_ack(back, ocall::fail_to_code(result.err()), ocall::EntryId::new(0));
            }
            ocall::TipIn::Empty { road, back } => {
                if !valid_tip_back(back, from) { continue; }
                let result = match tree.seek(&road) {
                    Ok(id) => tree.list(ocall::Where::At(id)).and_then(|children|
                        if children.count() == 0 { Ok(id) } else { Err(ocall::Fail::NonEmpty) })
                        .and_then(|id| tree.trim(id).map(|_| id)),
                    Err(ocall::Fail::Unknown) => Ok(ocall::EntryId::new(0)),
                    Err(fail) => Err(fail),
                };
                tip_ack(back, ocall::fail_to_code(result.err()), ocall::EntryId::new(0));
            }
            ocall::TipIn::Unplate { id, back } => {
                if !valid_tip_back(back, from) { continue; }
                let result = match tree.trim(id) { Err(ocall::Fail::Unknown) => Ok(None), other => other };
                if let Ok(Some(change)) = &result { answer::changed(tree, watchers, change); }
                tip_ack(back, ocall::fail_to_code(result.err()), id);
            }
            ocall::TipIn::Wired { authority, resolve, matches, same, back } => {
                if !valid_tip_back(back, from) {
                    continue;
                }
                *query = protocol::service::identity::client::TaskQuery::direct(
                    authority, resolve, matches, same,
                ).ok();
                if query.is_none() {
                    debug::put("operator: invalid identity query bundle");
                }
                tip_ack(back, if query.is_some() { ocall::OK } else { ocall::UNJUDGED }, ocall::EntryId::new(0));
            }
            // **一位客人**：按"它开的 + 记号"认它那条答话路。
            ocall::TipIn::Guest(client) => match reply_of(client) {
                Some(reply) => match desk.admit(client, reply) {
                    // 收了。
                    Ok(_) => {}
                    // **重放**（提示是单槽，可能重放）：一位客人只占一格，旧的那一格**原样留着**
                    Err(DeskFail::Already) => {}
                    // **满了**：这位客人进不来，而**它自己不知道**——它的问话孔没人管，第二次
                    Err(DeskFail::Full) => debug::put("operator: desk full"),
                },
                // 次序被破坏（提示先到、答话路不在本表里）：**别丢**——留在册上，下面逐轮重问。
                //
                // 丢掉的代价是那位客人**永远进不了这本账**（它的问话孔没人挂进组），它的每一问
                // 都压在孔上；而"没认到"与"这条路上根本没这枚孔"在结构上分得开：前者下一秒就好。
                // **把号一起报出来**：这一条与 `mail: hand stuck` **一一对应**（量过：全控制台
                // 194 份里各 3 份、同一批文件、无一例外），号是两条读数对得起来的唯一凭据。
                None => {
                    if !late.iter().any(|one| one.who == client) {
                        if late.try_reserve(1).is_err() {
                            // **备不下就是不收**：照实报，别把"没记"说成"记下了"。
                            debug::put(&alloc::format!(
                                "operator: no reply who={} kept=no",
                                client.get()
                            ));
                            continue;
                        }
                        debug::put(&alloc::format!(
                            "operator: no reply who={} kept=yes",
                            client.get()
                        ));
                        late.push(Late {
                            who: client,
                            since: now(),
                        });
                    }
                    pending = true;
                }
            },
        }
    }
    // **上一轮还没认到的那几位：逐轮重问**。认到就地收进来（与提示当场认到同一手 `admit`）；
    // 到点仍没有才放弃——那时报的是"等了多久"，与"没记"分得开。
    let mut at = 0;
    while at < late.len() {
        let who = late[at].who;
        if let Some(reply) = reply_of(who) {
            match desk.admit(who, reply) {
                Ok(_) | Err(DeskFail::Already) => {}
                Err(DeskFail::Full) => debug::put("operator: desk full"),
            }
            let ms = now().wrapping_sub(late[at].since) / 1_000_000;
            debug::put(&alloc::format!(
                "operator: late reply who={} after={}ms",
                who.get(),
                ms
            ));
            late.swap_remove(at);
            continue;
        }
        let ms = now().wrapping_sub(late[at].since) / 1_000_000;
        if ms >= LATE_MS as u64 {
            let one = late.swap_remove(at);
            debug::put(&alloc::format!(
                "operator: no reply who={} gave up after={}ms",
                one.who.get(),
                ms
            ));
            continue;
        }
        at += 1;
    }
    pending |= !late.is_empty();
    // 还没挂上问话孔的那几格：**账自己按格子号走一遍**（见 Desk::arm_pending）——
    // 调用方这一侧因此既不必按常数开数组（"一本账的容量渗到别人的栈上"那一格），
    // 也不必为"抄一份"再分配一次。
    // **记号那一列**（MARKS）：控制面那一枚（`ASK_MARK`）＋ 七位操作面各一枚。
    pending |= desk.arm_pending(
        &MARKS,
        |who, mark| ask_of(who, mark),
        |ask| {
            let ok = pile
                .attach(&mail::HolePie::from_token(ask), HoleDir::Pull)
                .is_ok();
            if ok && mail::HolePie::from_token(ask).peek().is_ok() {
                let owner = mail::reserve(ask)
                    .map(|(_, owner, _)| owner.get())
                    .unwrap_or(0);
                debug!("operator: arm late owner={} ask={}", owner, ask.get());
            }
            ok
        },
    );
    pending
}

/// **诊断（release 也看得见）**：把"还有人没挂上"那一档拆开——**是哪几位、`ask_of` 认不认得**
/// **为什么不用 `debug!`**：那一支宏在 release 下**是空操作**（`crates/protocol/src/debug.rs`
fn unarmed_report(desk: &Desk, rounds: usize) {
    let mut unarmed = 0usize;
    desk.unarmed_each(|_| unarmed += 1);
    if unarmed == 0 {
        return;
    }
    debug::put(&alloc::format!(
        "operator: settle guests={} unarmed={} ~{}ms",
        desk.occupied(),
        unarmed,
        rounds,
    ));
    // 逐位点名：**每位一行、每次至多四位**（病态时这是每 ~2 s 五行的量，不淹日志）。
    let mut said = 0usize;
    desk.unarmed_each(|who| {
        if said >= 4 {
            return;
        }
        said += 1;
        let hit = MARKS.iter().any(|mark| ask_of(who, *mark).is_some());
        debug::put(&alloc::format!(
            "operator: unarmed who={} ask={}",
            who.get(),
            if hit { "some" } else { "none" },
        ));
    });
}

struct Outbox {
    who: TaskId,
    send: Sender<ocall::Union>,
}

fn outbox<'a>(outs: &'a mut Vec<Outbox>, guest: Guest) -> Option<&'a mut Outbox> {
    if let Some(at) = outs.iter().position(|o| o.who == guest.who()) {
        return outs.get_mut(at);
    }
    outs.try_reserve(1).ok()?;
    outs.push(Outbox {
        who: guest.who(),
        send: Sender::<ocall::Union>::from_token(guest.reply()),
    });
    outs.last_mut()
}

/// 招待一位客人：从**它的问话孔**读一帧、交给树、把答话推进**它的答话路**
/// 组已经说了"这一枚有话"，故这一读读得动；期限给 `0` 是**再确认**，不是轮询
fn serve_one(
    tree: &mut Operator,
    watchers: &mut watch::Watchers,
    guest: Guest,
    query: Option<&protocol::service::identity::client::TaskQuery>,
    buf: &mut [u8],
    outs: &mut Vec<Outbox>,
) -> Option<u8> {
    let ask = guest.ask()?;
    // **收帧用调用方那一页**（Receiver::recv）：比家族最长那一枚更长的一条也取得出来、
    // 解得失败 ⇒ 照旧答一句 `BAD`，而槽也空了。
    // 收：**两格失败在这一门同一落点**（`answer` 收的还是 `Option`：读不懂与期限到了都答 `BAD`）。
    let decoded = match mail::HolePie::from_token(ask).pull(buf, Wait::POLL) {
        Ok((len, from)) if from == guest.who() => {
            <ocall::Req as protocol::wire::message::Message>::fetch(&buf[..len])
        }
        Ok((_, from)) => {
            debug!("operator: rejected session sender={} guest={}", from.get(), guest.who().get());
            return Some(0xff);
        }
        Err(_) => None,
    };
    // 这一问的动作码（首格）——**读到了才有可信的首格**；它同时就是"这一醒读到了没有"。
    let code = decoded.as_ref().map(|_| buf[0]);
    let grant = grant_of(mark_of(ask));
    let t_ans = runtime::env::chrono::clock();
    let said = answer(tree, watchers, decoded, guest.who(), query, grant);
    // 的那一格码**报出来——"哪一位客人、问什么（首格码）、答什么"三样齐了，才谈得上说得清。
    // **只在非 OK 时报**（正常一条答话不占串口）：`BAD` 与那六格各是一个成因。
    if let ocall::Union::Status(status) = said
        && status != ocall::OK
    {
        debug!(
            "operator: answered code={} ask={} who={}",
            status,
            code.unwrap_or(0xff),
            guest.who().get()
        );
    }
    // 答一句：**形状由 ocall::Union 说**——编进**这位客人那一格**的缓冲里、递出去（一个
    let sent = match outbox(outs, guest) {
        Some(out) => {
            if out.send.settle() {
                out.send.send(said).is_ok()
            } else {
                false
            }
        }
        _ => false,
    };
    let t_out = runtime::env::chrono::clock();
    let ms = |a: u64, b: u64| (b.saturating_sub(a) / 1_000_000) as usize;
    if ms(t_ans, t_out) >= 200 || !sent {
        protocol::debug::put(&alloc::format!(
            "operator: answer who={} answer={}ms sent={}",
            guest.who().get(),
            ms(t_ans, t_out),
            sent
        ));
    }
    code
}
