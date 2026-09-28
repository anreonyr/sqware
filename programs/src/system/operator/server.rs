//! operator::server — **持树者**：自己的域里的一枚线程守着那棵树（一枚线程 + 一个组，无轮询）。
//!
//! 本文件只放**那一枚线程**：起手（上板 → 铸提示孔交给装配者 → 一枚线程招待所有客人）与它收进来
//! 的四句话。每句话各自的正文在隔壁——**本文件不做裁决、也不动树**，它只"收一句、交给谁"：
//!
//! | 收的是什么 | 交给谁 |
//! |---|---|
//! | 提示之路上的一条路（[`ocall::TipIn::Plate`]） | [`super::plate::plate`]：前缀立窗格 ＋ 末段落格 |
//! | 提示之路上的协调两格（[`ocall::TipIn::Coord`]） | 本文件那一格 `coord`（门**问的时候**才认，见 [`super::door`]） |
//! | 提示之路上的一位客人（[`ocall::TipIn::Guest`]） | 客人账（`Desk::admit`） |
//! | 客人的一句问（[`ocall::Req`]） | [`super::answer::answer`]：七条原语 |
//!
//! **为什么就一枚线程**：树上那几枚是**一枚只在持它的那张表里有意义的句柄**（`PieToken` = "我这张
//! 表里的第几个"）。"查到了要把 Pie 授出去"必须由**持有那一枚的那张表**来做——故所有条目只能住
//! 同一张表，也就是同一枚线程。板那一台栽过这条（每位客人一枚待客线程 ⇒ 甲的条目在甲的表里，
//! 乙来查时判它"已死"、也授不出去，症状是"刚挂上的名字，别人一查就是 `Unknown`"）。
//!
//! **三侧分家**：两侧共用的图与说明见 [`super`] 的"载体"那一节，帧与记号见
//! [`protocol::system::operator`]。
//!
//! **照实记（这一份没有 task-2 那一刀的迁移点）**：`operator::client` 新出那一面是**客侧**用的；
//! 本文件是持树者，一处客手都不叫（它自己那几手在 `core` 与 `plate`/`answer`/`door` 里，帧从门
//! 闩直接读）。故"已持 Session 则用 Face"这条规则在这里落成一句"不适用"——写下来备查，免得下
//! 一刀再来找一遍。

use env::wire::Eyes;
use env::{HoleDir, Mark, Wait};
use runtime::PAGE_SIZE;
use runtime::core::pile::Pile;
use runtime::core::port::{self, Access, Policy};
use runtime::env::mail;

use protocol::communication::receiver::Receiver;
use protocol::communication::sender::Sender;
use protocol::debug;
use protocol::system::operator as ocall;
use protocol::system::operator::grant::grant_of;
use protocol::system::operator::Grant;

use crate::system::control::READY_MS;
use crate::system::control::service::Start;
use crate::program::operator::E_TREE;
use crate::system::desk::{Desk, DeskFail, Guest};
use crate::system::operator::core::Operator;

use super::answer::answer;
use super::bridge::Coord;
use super::claim::{ask_of, mark_of, reply_of};
use super::plate::plate;

/// 还在"补齐两本账"（答话路未认领 / 问话孔未挂上）时，一轮等多久（毫秒）。
///
/// **不是轮询**：账补齐之后这一等就变成 `Wait::Forever`（由组唤醒）；这个短期限只在装配窗口
/// 里用——那几步的到达是**别人**在做（装配者转授、客人自己交孔）。
const SETTLE_MS: usize = 1;

/// **本族认得的全部问话孔记号**：控制面那一枚 ＋ 七位操作面各一枚。
///
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
///
/// 装配者要本域做的几件事都从**提示孔**那一条路进来（立一条路 / 协调两格 / 一位客人，见
/// [`settle`]）：它是"装配侧 → 持树者"的唯一一条路，故**不必另开一条到自己的会话**。
///
/// 头两步是契约：装配者按 `(本域, tip)` 两格认领提示孔（`bridge::host_of`），而提示一到它就认为
/// "答话路必已在本表里"（转授在前、提示在后）。
pub fn serve() -> Result<(), Start> {
    // **起我那一枚线程**：本域是装配者建的，故 `Sire` 答的就是它——**只有这一条来源**
    // （从 `args` 里掏一格那条绕路已退：它存在只因为 iii 让三枚与编排者同域）。
    let assembler = runtime::env::unit::sire();
    // **上板**：让板看得见**本域（这一枚线程）的死**——三枚内件此后同形
    // （名册 / 盟册早就在上板）。**名字不必本域自己报名**：装配者随提示那一格递过来；
    // 这一格只管把板那条路装上（装配者那一侧要按 `(本域, 板路)` 认领本域交出去的那一枚，
    // 故少了这一步装配当场报 `board:claim`）。
    let _board = match protocol::communication::session::Session::open(
        assembler,
        crate::system::board::client::BERTH,
        Wait::AtMost(READY_MS),
    ) {
        Ok(seat) => seat,
        Err(_) => return Err(Start::Board(E_TREE)),
    };
    // 提示孔：本线程铸的那一枚（**装配者要它做的三件事都从这里进来**：立一条路 / 协调两格 /
    // 一位客人），副本交给生我者。**记号 = `tip`**。
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

    let mut tree = Operator::new();
    let mut desk = Desk::new();
    // 协调那一帧递来的两格号（名册 / 盟册，各自那一域自己把门牌交过来）。**两帧、次序不定**。
    //
    // **本域不在这里认门牌**（照实记：这一刀换过一格）：认那一手住在门口
    // （[`super::door::may`]）——问到门上才认，于是"门那一侧的状态"不再寄在"路那一侧"身上。
    let mut coord = Coord::default();
    // **那本账没了**（照实记）：归属从前另住一本（`Book`），它自称"活着那一问与树叫的是同一具
    // 身体"——正因为是同一句，它只能是树的影子：`land` 那一趟把同一个 `PieToken` 同时交给两处，
    // 而 `Line` 那一行里另外三格（`name` / `id` / `at`）树上本来就有。影子撤掉，两轴都跟着砖走。
    // 收帧的那一页：**在循环外备一次**——门的缓冲不再是"这一族最大的那一帧"（`REQ_LEN`），
    // 而是**载体的一页**：界判在 `Push`，故客人推得进来的最长就是一页。
    let mut buf: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    if buf.try_reserve_exact(PAGE_SIZE).is_err() {
        return Err(Start::Room(E_TREE));
    }
    buf.resize(PAGE_SIZE, 0);
    loop {
        // 一、补齐那几件事（收提示之路上那三种帧；认领答话路；认出问话孔并挂组）。
        let settling = settle(&mut desk, &pile, &tip_hole, &mut coord, &mut tree);
        // 二、等一格有事。**一个等待**：提示孔或任意一位客人的问话孔。
        let millis = if settling {
            Wait::AtMost(SETTLE_MS)
        } else {
            Wait::Forever
        };
        let Ok(Some((tok, _dir))) = pile.await_(millis) else {
            let _ = desk.sweep();
            continue;
        };
        // 提示孔那一格由下一轮的 `settle` 收（它非阻塞地拉）；这里只管"是哪位客人的问话孔"。
        if tok != tip
            && let Some(guest) = desk.guest(tok).copied()
        {
            serve_one(&mut tree, guest, coord, &mut buf);
        }
        // 三、**看出来的**那一档：那一枚答不出 ⇒ 剔格子（没有"他说走了"那一档）。
        let _ = desk.sweep();
    }
}

/// 补齐那几件事，返"还有没有没补齐的"。
///
/// - **提示之路**：装配者推来的三形，**首格 `kind` 分派**（见 [`ocall::TipIn`]）：
///   - **一条路**（[`ocall::TipIn::Plate`]）：装配者要本域在树上立一条路——**本域自己立**
///     （[`plate`]），不经会话、不当自己的客人；
///   - **协调两格**（[`ocall::TipIn::Coord`]）：装配者把"**哪一位域** + **它是哪一双眼睛**"
///     直接递过来。两帧、次序不定；**本域只记号**，认门牌那一手在门口（问的时候才认）；
///   - **一位客人**（[`ocall::TipIn::Guest`]）：`admit` 收进来。
///
///   **非阻塞地拉**——必须在这里拉，不能只在"组唤醒"那一支拉：装配者的推**可能早于本线程把
///   提示孔挂进组**（那一条推落在一个还没有转发登记的站点上），醒不来就得靠这一拉吃到它；
/// - **答话路**：装配者转授来的那一枚 ⇒ `admit` 收一位客人；
/// - **问话孔**：客人**自己**交来的那一枚 ⇒ 认出来就 `arm` + 挂进组。
fn settle(
    desk: &mut Desk,
    pile: &Pile,
    tip: &mail::HolePie,
    coord: &mut Coord,
    tree: &mut Operator,
) -> bool {
    // 提示：拉干净（单槽，一位客人一条）。**非阻塞**——它的到达是别人在做的事。
    // 缓冲按**最长那一形**备（立一条路），故另两形也吃得下——小缓冲会把长帧读成"读不懂"。
    let mut frame = [0u8; ocall::TIP_LEN];
    let mut pending = false;
    loop {
        let Ok(n) = tip.pull_timeout(&mut frame, Wait::POLL) else {
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
            ocall::TipIn::Plate {
                road,
                count,
                leaf,
                rule,
            } => plate(tree, &road[..count], leaf, rule),
            // **一双眼睛**：两格——哪一位域、它是哪一双眼睛。各自那一枚门牌由那一域**自己**交
            // 进来（装配者只递号）；从这里往后门禁问得动身份（认那一手在 [`super::door`]）。
            ocall::TipIn::Coord { who, eyes } => match eyes {
                Eyes::Roster => coord.roster = Some(who),
                Eyes::League => coord.league = Some(who),
            },
            // **一位客人**：按"它开的 + 记号"认它那条答话路。
            ocall::TipIn::Guest(client) => match reply_of(client) {
                Some(reply) => match desk.admit(client, reply) {
                    // 收了。
                    Ok(_) => {}
                    // **重放**（提示是单槽，可能重放）：一位客人只占一格，旧的那一格**原样留着**
                    // ——这一趟不动账，也不打行（重放是常态）。
                    Err(DeskFail::Already) => {}
                    // **满了**：这位客人进不来，而**它自己不知道**——它的问话孔没人管，第二次
                    // 问话会堵在单槽上（整台机器收不了场）。故这一格**报一句，别静默丢一位客人**。
                    Err(DeskFail::Full) => debug!("operator: desk full"),
                },
                // 次序被破坏（提示先到、答话路不在本表里）：报一句；客人那边会报它自己的超时。
                None => debug!("operator: no reply"),
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
            pile.attach(&mail::HolePie::from_token(ask), HoleDir::Pull)
                .is_ok()
        },
    );
    pending
}

/// 招待一位客人：从**它的问话孔**读一帧、交给树、把答话推进**它的答话路**。
///
/// 组已经说了"这一枚有话"，故这一读读得动；期限给 `0` 是**再确认**，不是轮询。
///
/// `buf` = **调用方那一页**（`serve` 在循环外备一次）：收帧不在这里分配，也不是家族帧那么大
/// ——客人的最长帧由载体定（一页），门就得有一页才接得住。
///
/// **这一位叫的是哪一条原语**：从**本域表里那枚问话孔**的记号读回（客户端自称不了，见
/// `grant_of`）。认不出 = 会话没说它持哪一柄权（控制面那条路）⇒ `None` ⇒ 不判面。
fn serve_one(
    tree: &mut Operator,
    guest: Guest,
    coord: Coord,
    buf: &mut [u8],
) {
    let Some(ask) = guest.ask() else {
        return;
    };
    // **收帧用调用方那一页**（`Receiver::recv`）：比家族最长那一枚更长的一条也取得出来、
    // 解得失败 ⇒ 照旧答一句 `BAD`，而槽也空了。
    // 收：**两格失败在这一门同一落点**（`answer` 收的还是 `Option`：读不懂与期限到了都答 `BAD`）。
    let decoded = Receiver::<ocall::Req<'_>>::from_token(ask)
        .recv(buf, Wait::POLL)
        .ok();
    let grant = grant_of(mark_of(ask));
    let said = answer(tree, decoded, guest.who(), coord, grant);
    // 答一句：**形状由 [`ocall::Union`] 说**——装与发都不在这一层写字节。
    // `.ok()`：装不上那一格按构造到不了（`Buf` 由本族 `Message` 自己给，见 `Sender::send`）。
    let _ = Sender::<ocall::Union>::from_token(guest.reply())
        .send(said, Wait::Forever)
        .ok();
}
