//! operator::server — **持树者**：自己的域里的一枚线程守着那棵树（一枚线程 + 一个组，无轮询）。
//! 本文件只放**那一枚线程**：起手（上板 → 铸提示孔交给装配者 → 一枚线程招待所有客人）与它收进来
//! 的四句话。每句话各自的正文在隔壁——**本文件不做裁决、也不动树**，它只"收一句、交给谁"：
//! | 收的是什么 | 交给谁 |
//! |---|---|
//! | 提示之路上的一条路（[`ocall::TipIn::Plate`]） | [`super::plate::plate`]：前缀立窗格 ＋ 末段落格 |
//! | 提示之路上的一位客人（[`ocall::TipIn::Guest`]） | 客人账（`Desk::admit`） |
//! | 客人的一句问（[`ocall::Req`]） | [`super::answer::answer`]：七条原语 |
//! **为什么就一枚线程**：树上那几枚是**一枚只在持它的那张表里有意义的句柄**（`PieToken` = "我这张
//! 表里的第几个"）。"查到了要把 Pie 授出去"必须由**持有那一枚的那张表**来做——故所有条目只能住
//! 同一张表，也就是同一枚线程。板那一台栽过这条（每位客人一枚待客线程 ⇒ 甲的条目在甲的表里，
//! 乙来查时判它"已死"、也授不出去，症状是"刚挂上的名字，别人一查就是 `Unknown`"）。
//! **三侧分家**：两侧共用的图与说明见 [`super`] 的"载体"那一节，帧与记号见
//! [`protocol::service::operator`]。

use alloc::vec::Vec;
use env::{HoleDir, Mark, PieToken, TaskId, Wait};
use runtime::PAGE_SIZE;
use runtime::core::pile::Pile;
use runtime::core::port::{self, Access, Policy};
use runtime::env::mail;

use protocol::communication::receiver::{Receiver, RecvFail};
use protocol::communication::sender::Sender;
use protocol::debug;
use protocol::service::operator as ocall;
use protocol::service::operator::Grant;
use protocol::service::operator::grant::grant_of;

use crate::service::operator::core::Operator;
use crate::system::control::service::Start;
use crate::system::desk::{Desk, DeskFail, Guest};
use crate::unit::operator::E_TREE;

use super::answer::answer;
use super::claim::{ask_of, mark_of, reply_of};
use super::plate::plate;

/// 还在"补齐两本账"（答话路未认领 / 问话孔未挂上）时，一轮等多久（毫秒）。
/// **不是轮询**：账补齐之后这一等就变成 `Wait::Forever`（由组唤醒）；这个短期限只在装配窗口
/// 里用——那几步的到达是**别人**在做（装配者转授、客人自己交孔）。
const SETTLE_MS: usize = 1;

/// **本族认得的全部问话孔记号**：控制面那一枚 ＋ 七位操作面各一枚。
/// **一处给**：[`Desk::arm_pending`] 逐枚试、[`ask_of`] 逐枚比——两处读的都只有这一个数组。
/// 位次与记号的对照本体在 [`Grant`]（`Grant::mark`），这里只是把它摊平。
const MARKS: [Mark; 8] = [
    ocall::ASK_MARK,
    Grant::Part.mark(),
    Grant::Land.mark(),
    Grant::Find.mark(),
    Grant::Trim.mark(),
    Grant::List.mark(),
    Grant::Seek.mark(),
    Grant::Name.mark(),
];

/// 起服务：**上板 → 铸提示孔交给装配者 → 一枚线程招待所有客人**。
/// 装配者要本域做的几件事都从**提示孔**那一条路进来（立一条路 / 协调两格 / 一位客人，见
/// [`settle`]）：它是"装配侧 → 持树者"的唯一一条路，故**不必另开一条到自己的会话**。
/// 头两步是契约：装配者按 `(本域, tip)` 两格认领提示孔（`bridge::host_of`），而提示一到它就认为
/// "答话路必已在本表里"（转授在前、提示在后）。
pub fn serve() -> Result<(), Start> {
    // **起我那一枚线程**：本域是装配者建的，故 `Sire` 答的就是它——**只有这一条来源**
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

    // **报"答得动了"**（`Setup::Ready`）：上面那一趟落完面、查回来验过才算——被 `after` 指着的台
    // 必须说得出这一句（与三台驱动、设备账那两处**同一手**，见 `programs/src/unit/catalog.rs` 那一格）。
    let _ = protocol::communication::establish::endpoint(
        runtime::env::unit::sire(),
        env::Mark::of(crate::unit::READY),
        env::Wait::POLL,
    );

    let mut tree = Operator::new();
    let mut desk = Desk::new();
    let mut wired = false;
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
        // 一、补齐那几件事（收提示之路上那三种帧；认领答话路；认出问话孔并挂组）。
        let settling = settle(&mut desk, &pile, &tip_hole, &mut wired, &mut tree);
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
        // 提示孔那一格由下一轮的 `settle` 收（它非阻塞地拉）；这里只管"是哪位客人的问话孔"。
        // `serve_one` 返的是**这一问的动作码**（首格，`None` = 什么也没读到）——"读到没有"与
        // "要的是什么"是同一趟的两个事实，故一处交出来（见它那一节）。
        let code = if tok != tip
            && let Some(guest) = desk.guest(tok).copied()
        {
            serve_one(&mut tree, guest, wired, &mut buf, &mut outs)
        } else {
            // **醒在一枚账上没有的号上**（守卫）：提示孔是常态（它的帧由下一轮的 `settle` 收），
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
        // 来的成员（且每剔一位多留一格）。**不封印**：那一枚是**客人铸的**，本域只持副本。
        let _ = desk.sweep_each(|gone| {
            if let Some(ask) = gone.ask {
                let _ = pile.detach(&mail::HolePie::from_token(ask), HoleDir::Pull);
            }
            // **答话那一格也一起办**：先非阻塞地问一句"上一答被取走了没有"（[`Sender::settle`]）
            if let Some(at) = outs.iter().position(|o| o.who == gone.who)
                && outs[at].send.settle()
            {
                let _ = outs.swap_remove(at);
            }
        });
    }
}

/// 补齐那几件事，返"还有没有没补齐的"。
/// - **提示之路**：装配者推来的三形，**首格 `kind` 分派**（见 [`ocall::TipIn`]）：
///   - **一条路**（[`ocall::TipIn::Plate`]）：装配者要本域在树上立一条路——**本域自己立**
///     （[`plate`]），不经会话、不当自己的客人；
///   - **一位客人**（[`ocall::TipIn::Guest`]）：`admit` 收进来；
///   - **门禁接线**（[`ocall::TipIn::Wired`]）：**一句话、不带号**——装配者已认下名册，门从此
///     问得动身份（那一格由 [`super::door::may`] 读）。
fn settle(
    desk: &mut Desk,
    pile: &Pile,
    tip: &mail::HolePie,
    wired: &mut bool,
    tree: &mut Operator,
) -> bool {
    // 提示：拉干净（单槽，一位客人一条）。**非阻塞**——它的到达是别人在做的事。
    // 缓冲按**最长那一形**备（立一条路），故另两形也吃得下——小缓冲会把长帧读成"读不懂"。
    let mut frame = [0u8; ocall::TIP_LEN];
    let mut pending = false;
    loop {
        let Ok((n, _)) = tip.pull(&mut frame, Wait::POLL) else {
            break;
        };
        // **首格 `kind` 决定形状**：表外的 kind / 长度不对 ⇒ 读不懂。这条路上没有答话那一格，
        // 故只能**报一句**（把那一格 kind 一起报出来，"读不懂的是哪一形"要看得见）。
        let Some(rec) = ocall::TipIn::fetch(&frame[..n]) else {
            debug!("operator: tip unreadable (kind={})", frame[0]);
            continue;
        };
        match rec {
            // **装配者要本域立一条路**。
            ocall::TipIn::Plate { road, leaf, rule } => plate(tree, &road, leaf, rule),
            ocall::TipIn::Wired => *wired = true,
            // **一位客人**：按"它开的 + 记号"认它那条答话路。
            ocall::TipIn::Guest(client) => match reply_of(client) {
                Some(reply) => match desk.admit(client, reply) {
                    // 收了。
                    Ok(_) => {}
                    // **重放**（提示是单槽，可能重放）：一位客人只占一格，旧的那一格**原样留着**
                    // ——这一趟不动账，也不打行（重放是常态）。
                    Err(DeskFail::Already) => {}
                    // **满了**：这位客人进不来，而**它自己不知道**——它的问话孔没人管，第二次
                    Err(DeskFail::Full) => debug::put("operator: desk full"),
                },
                // 次序被破坏（提示先到、答话路不在本表里）：报一句；客人那边会报它自己的超时。
                None => debug::put("operator: no reply"),
            },
        }
    }
    // 还没挂上问话孔的那几格：**账自己按格子号走一遍**（见 [`Desk::arm_pending`]）——
    // 调用方这一侧因此既不必按常数开数组（"一本账的容量渗到别人的栈上"那一格），
    // 也不必为"抄一份"再分配一次。
    // **记号那一列**（[`MARKS`]）：控制面那一枚（`ASK_MARK`）＋ 七位操作面各一枚。
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

/// **诊断（release 也看得见）**：把"还有人没挂上"那一档拆开——**是哪几位、`ask_of` 认不认得**。
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

/// **一位客人一格答话存根**：那一枚答话路的写端 ＋ 这一族最长那一帧的缓冲（[`Sender`] 自带）。
struct Outbox {
    who: TaskId,
    send: Sender<ocall::Union>,
}

/// 取这位客人那一格答话存根（没有就按需立一格）。**备不下 ⇒ `None`**：那一趟不答
/// （宁可少答一句，也不让本域为它停住）。
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

/// 招待一位客人：从**它的问话孔**读一帧、交给树、把答话推进**它的答话路**。
/// 组已经说了"这一枚有话"，故这一读读得动；期限给 `0` 是**再确认**，不是轮询。
/// `buf` = **调用方那一页**（`serve` 在循环外备一次）：收帧不在这里分配，也不是家族帧那么大
/// ——客人的最长帧由载体定（一页），门就得有一页才接得住。
/// **这一位叫的是哪一条原语**：从**本域表里那枚问话孔**的记号读回（客户端自称不了，见
/// `grant_of`）。认不出 = 会话没说它持哪一柄权（控制面那条路）⇒ `None` ⇒ 不判面。
fn serve_one(
    tree: &mut Operator,
    guest: Guest,
    wired: bool,
    buf: &mut [u8],
    outs: &mut Vec<Outbox>,
) -> Option<u8> {
    let ask = guest.ask()?;
    // **收帧用调用方那一页**（`Receiver::recv`）：比家族最长那一枚更长的一条也取得出来、
    // 解得失败 ⇒ 照旧答一句 `BAD`，而槽也空了。
    // 收：**两格失败在这一门同一落点**（`answer` 收的还是 `Option`：读不懂与期限到了都答 `BAD`）。
    let decoded = match Receiver::<ocall::Req>::from_token(ask).recv(buf, Wait::POLL) {
        Ok(wire) => Some(wire),
        // ①③：没有手 / 孔用不动了。**域码原样报**（`Busy` 与 `Dead` / `Denied` 是两件事）。
        Err(RecvFail::Mail(e)) => {
            debug!(
                "operator: ask empty who={} tok={} code={}",
                guest.who().get(),
                ask.get(),
                e.code()
            );
            None
        }
        Err(RecvFail::Unread(len)) => {
            let show = len.min(8);
            let mut head = [0u8; 8];
            if let Some(src) = buf.get(..show) {
                head[..show].copy_from_slice(src);
            }
            debug!(
                "operator: ask unreadable who={} tok={} len={} head={:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x}",
                guest.who().get(),
                ask.get(),
                len,
                head[0],
                head[1],
                head[2],
                head[3],
                head[4],
                head[5],
                head[6],
                head[7]
            );
            None
        }
    };
    // 这一问的动作码（首格）——**读到了才有可信的首格**；它同时就是"这一醒读到了没有"。
    let code = decoded.as_ref().map(|_| buf[0]);
    let grant = grant_of(mark_of(ask));
    let t_ans = runtime::env::chrono::clock();
    let said = answer(tree, decoded, guest.who(), wired, grant);
    // **答的是什么**：客侧把"忙 / 没有 / 读不懂"折成同一格（`Unknown`），故这一侧要把**本域答出去
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
    // 答一句：**形状由 [`ocall::Union`] 说**——编进**这位客人那一格**的缓冲里、递出去（一个
    // envcall），**编完就接着招待下一位**。**孔是客人铸的**：`release` 那一手不在本域做
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
    // 读数（诊断，release 也报）：这一趟整段有没有停下来 / 有没有少答一句。
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
