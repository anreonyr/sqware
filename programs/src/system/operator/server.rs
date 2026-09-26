//! operator::server — **持树者**：自己的域里的一枚线程守着那棵树（一枚线程 + 一个组，无轮询）
//!
//! 三侧分家之后本文件只放**持树者**：自己的域里的一枚线程守着那棵树（一枚线程 + 一个组，无轮询）；两侧共用的图与说明见 [`super`] 的"载体"那一节，
//! 帧与记号见 [`protocol::system::operator`]。

use env::wire::Field;
use env::Wait;
use plan::assembly::Eyes;
use env::{HoleDir, Mark, PieToken, TaskId};
use runtime::core::port::{self, Access, Policy};
use runtime::core::pile::Pile;
use runtime::env::mail;
use runtime::PAGE_SIZE;

use protocol::system::operator as ocall;
use protocol::session::slip::Slip;
use protocol::system::operator::core::gate::{Code, Control, verdict};
use protocol::system::operator::core::judge::{Id, Rule};
use protocol::system::operator::core::ledger::{Key, Ledger};
pub use protocol::system::operator::{ASK_MARK, LINK, TIP_MARK};
use protocol::system::operator::{EntryId, Fail, Listing, Operator, Where};
use protocol::system::board as bcall;
use protocol::system::board::client as board;

use protocol::system::coalition::client::Face as CoalitionFace;
use protocol::system::principal::client::Face as PrincipalFace;
use protocol::system::principal::core::PrincipalId;

use super::bridge::Coord;
use contract::system::desk::{Desk, DeskFail, Guest};
use protocol::system::operator::desk;


/// 还在"补齐两本账"（答话路未认领 / 问话孔未挂上）时，一轮等多久（毫秒）。
///
/// **不是轮询**：账补齐之后这一等就变成 `Wait::Forever`（由组唤醒）；这个短期限只在装配窗口
/// 里用——那几步的到达是**别人**在做（装配者转授、客人自己交孔）。
const SETTLE_MS: usize = 1;

// ── 门外那一问（门禁）────────────────────────────────────────

/// **两枚门牌**：身份服务那一枚（答"这一位此刻代表谁"与"在不在他那一支里"）与盟册服务那一枚
/// （答"这一位在那枚盟里吗"）。
///
/// 两枚都是装配者**递一格号**、由各自那一域**自己** `ship` 进来的（见
/// `contract/src/system/operator/frame.rs` 的 `CoordFrame` 照实记：装配者转授那一版真机
/// 栽在 `coord-ship`）。树**不当自己的客人**：它不去 `seek("/sys/principal")`，理由同那一笔
/// （自指 ⇒ 环）。
///
/// **盟册那一枚是 `Option`**：它晚到（或压根没配上）时，只有 [`Rule::In`] 那一格答"判不了"
/// （`Unjudged` 的"会好"那一类——补一帧就好），其余照旧。**降级是诚实的，不是放行**。
struct Session {
    roster: PrincipalFace,
    league: Option<CoalitionFace>,
}

impl Session {
    /// 认出那两枚门牌：**按"谁开的 + 记号"两格**在本表里找（协调那一帧只带号）。
    ///
    /// 两格都是确定的：那扇门是**各自那一域**开的（副本共享同一事实），记号 = 服务入口记号
    /// （`bcall::ENTRY_MARK`）。**不必装配者转授**——各域自己在 `serve_tree` 之后把它直接交给
    /// 持树者（见 `CoordFrame` 的照实记）。
    ///
    /// 名册那枚是契约：没有它就没有门禁，故它认不出 ⇒ 整格 `None`（读数会喊一句）。盟册那枚
    /// 认不出 ⇒ 只少 `Rule::In` 那一格。
    fn of(coord: Coord) -> Option<Session> {
        let roster = PrincipalFace::of(find_face(coord.roster?)?).ok()?;
        let league = coord
            .league
            .and_then(find_face)
            .and_then(|token| CoalitionFace::of(token).ok());
        Some(Session { roster, league })
    }
}

/// 找**某一位域**交给本域的那枚服务门牌（`opened_by == who` 且记号是服务入口）。
///
/// 认领的规矩与另外两处同一句（见 [`claim`]）；门牌那一枚走的是**裸 `unseal_hole`**，
/// 故"一个域只交一枚"同样是纪律而不是判据。
fn find_face(who: TaskId) -> Option<PieToken> {
    // 多枚**正常**（副本共享 `opened_by`：`land` 交一枚、门禁交一枚）⇒ 不说。
    claim(bcall::ENTRY_MARK, who, None)
}

/// 门禁要的那几条边都从这一份出：问身份（`resolve`）、谱系（`heir`）、盟籍（`amid`），
/// 外加**树自己**那一问（第 `n` 格是谁的门牌）。
///
/// 照实记（第五格那一刀）：`Session` 只拿两枚门牌，而树不住它里面（`&mut` 那一条借用过不去），
/// 故这一格把**两半**凑在一起——名册/盟册（[`Session`]）+ 树（[`Operator`]）。名字取"那一问在
/// 哪儿答"，与 [`operator::core::gate`](protocol::system::operator::core::gate) 的裁决/门禁一族同调。
struct Court<'a> {
    session: &'a Session,
    tree: &'a Operator,
}

impl Control for Court<'_> {
    fn who(&self, tid: TaskId) -> Result<Option<Id>, ()> {
        match self.session.roster.resolve(tid, Wait::AtMost(MS)) {
            // **不截断**：号在模型里的宽度就是 8 字节（`judge::Id`）。照实记：这里原写的是
            // `p.get() as u32`——号不上帧时看不出来，那一刀之后号要上帧，故一并提宽。
            Ok(found) => Ok(found.map(|p| p.get() as Id)),
            Err(_) => Err(()),
        }
    }

    fn heir(&self, a: Id, b: Id) -> Result<bool, ()> {
        self.session
            .roster
            .heir(
                PrincipalId::new(a as usize),
                PrincipalId::new(b as usize),
                Wait::AtMost(MS),
            )
            .map_err(|_| ())
    }

    fn amid(&self, me: Id, at: Id) -> Result<bool, ()> {
        // 盟册那一枚没在手里 ⇒ 答"问不到"，而不是答"否"——**判不了**与"不在那枚盟里"是
        // 两件事，后者会让客人当场放弃。
        let Some(league) = self.session.league.as_ref() else {
            return Err(());
        };
        league
            .amid(
                PrincipalId::new(me as usize),
                protocol::system::coalition::CoalitionId::new(at as usize),
                Wait::AtMost(MS),
            )
            .map_err(|_| ())
    }

    fn opens(&self, at: EntryId) -> Result<Option<TaskId>, ()> {
        // **判据要的只有"有没有那一位"**（见 `judge::Door`），故三种"没有"在裁决那一侧同落
        // `Ok(None)`。这一条**不动树**：剔死是 `find` 的活儿。
        //
        // **照实记（这一刀）**：三因同落是**判据**要的，但**后两因永远好不了**，而它们与
        // "对面暂时不答"在线上同落 `UNJUDGED` ⇒ 客人看不见差别（也对：下一步都是当趟放弃）。
        // 故这里**不压成一句 `.ok()`**：每一因各说一行读数——要的就是"为什么判不了"。
        match self.tree.opens(at) {
            Ok(tid) => Ok(Some(tid)),
            // 碑 / 从没铸过：**永久**。
            Err(Fail::Unknown) => {
                say(&alloc::format!("operator: opens gone n={}", at.get()));
                Ok(None)
            }
            // 那一号是块窗格：**永久**（结构事实）。
            Err(Fail::NotATile) => {
                say(&alloc::format!("operator: opens pane n={}", at.get()));
                Ok(None)
            }
            // 开者答不出——**永久**。`Dead` 自己仍是**三因一码**（不是孔 / 不在我表里 / 已封印，
            // 见 [`Fail::Dead`]）：这一行读数是"没有开者"，不声称分得开那三因。
            Err(Fail::Dead) => {
                say(&alloc::format!("operator: opens sealed n={}", at.get()));
                Ok(None)
            }
            // 余下三格**到不了**（`core::opens` 的判据表只有上面三条）。一格一格列出来，是为了
            // 将来 `Fail` 多一格时**编不过**，而不是悄悄落进一个 `_`。
            Err(Fail::NonEmpty | Fail::NotAPane | Fail::Full) => Ok(None),
        }
    }
}

/// **门禁的入口**：`session` 为 `None` ⇒ **放行**；`Some` ⇒ 按那一格自己的规矩判
/// （[`Ledger::rule`] 答出来的那一条）。
///
/// **照实记（这一格原来挂着一条"不许放行"的裁决）**：本节原写、并曾记成已定的是
/// 「没有门牌时客人请求答 `UNJUDGED`，**不许**答 `Allow`」。实现是放行，而它
/// **不是**门禁的例外，是**装配期根本不在门禁这条轴上**：
///
/// principal 挂自己门牌那一趟（`part /sys` + `land /sys/principal`）发生在它的 `serve_tree`
/// 里，而本域**认下它的门牌**与它**拿到身份**（`derive(ROOT)` + `bind`）都在**那之后**
/// （`service.rs` 的 `assemble` 里那两段）⇒ 那一刻它**既没有门牌、又还没有身份**。门禁若在
/// 那一刻生效，它连自己的门牌都挂不上，整机起不来。运行期那些客人则都在 `Hatch` 放行之前
/// 拿到了身份——**挡门的是它们，不是装配这一步**。
///
/// `tree` 只给第 `n` 格那一问（`Rule::Opens`）：判据要问"那一格是谁的门牌"，而树就在手里
/// ——故它一并与门牌合成 [`Court`]。
fn may(tree: &Operator, session: Option<&Session>, who: TaskId, rule: Rule<Id, Id>) -> Code {
    match session {
        None => Code::Ok,
        Some(s) => match verdict(&Court { session: s, tree }, who, rule) {
            // 「手里没有门牌」在客人那一侧与「判不了」同一格；`Some` 的时候不该出现它
            // （`Court` 不覆盖 `has_face`），真出现了也按"判不了"走，不按"放行"。
            Code::Blind => Code::Unjudged,
            other => other,
        },
    }
}

/// **账对真相的那一问**：那一号此刻还是**一枚 `Tile`** 吗。
///
/// 陈旧的三种样子一次答完（见 `ledger.rs` 那张表）：
///
/// - 还在、还是 `Tile` ⇒ 真；
/// - `trim` 剪掉 / `find` 剔死 ⇒ `name` 答 `Unknown` ⇒ 假；
/// - `part` 把它顶成一块 `Pane` ⇒ `list` 答得出 ⇒ 假。
///
/// 它只在"账要拒"的那一支被叫（账是缓存），故这一趟读不落在热路上。
fn fresh(tree: &Operator, id: EntryId) -> bool {
    tree.name(id).is_ok() && tree.list(Where::At(id)).is_err()
}

/// 问身份那两条边要用的期限（毫秒）。**必须有界**：协调服务不在时不能把树挂死。
const MS: usize = 1000;

// ── 持树者侧（本域的服务线程）────────────────────────────────

/// 起服务：**上板 → 铸提示孔交给装配者 → 一枚线程招待所有客人**。
///
/// 头两步是契约：装配者按 `(本域, tip)` 两格认领提示孔（[`attach`] 的 `host_of`），
/// 而提示一到它就认为"答话路必已在本表里"（转授在前、提示在后）。
pub fn serve() -> Result<(), super::fail::Fail> {
    // **起我那一枚线程**（不是 `sire()`：那一手答的是**域级**的生我者，对住本域的
    // 这一枚指的不是编排者。见 `service::Role::args` 的照实记）。
    let Some(assembler) = crate::service::assembler() else {
        return Err(super::fail::Fail::Sire);
    };
    // **上板**（乙那一刀）：让板看得见**本域（这一枚线程）的死**——三枚内件此后同形
    // （名册 / 盟册早就在上板）。**名字不必本域自己报名**：装配者随提示那一格递过来
    // （[`protocol::system::board::frame::Tip::LEN`] 的照实记）；这一格只管把板那条路装上
    // （装配者那一侧要按 `(本域, 板路)` 认领本域交出去的那一枚，故少了这一步装配当场报
    // `board:claim`——实测栽过一次）。
    let Ok((_link, board_link)) = board::open(assembler, Wait::AtMost(MS)) else {
        return Err(super::fail::Fail::Board);
    };
    if board::ask_hole(board_link).is_err() {
        return Err(super::fail::Fail::Board);
    }
    // 提示孔：本线程铸的那一枚（客人号从这里进来），副本交给生我者。**记号 = `tip`**。
    let Ok(tip) = mail::unseal_hole(TIP_MARK) else {
        return Err(super::fail::Fail::Tip);
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
        return Err(super::fail::Fail::Tip);
    }
    // **一个组**：提示孔 + 每位客人的问话孔。提示孔也挂进来，故"来客人了"与"有人问话"
    // 是**同一个等待**。本线程独享它（`shared = false`）。
    let Ok(pile) = Pile::unseal(false) else {
        return Err(super::fail::Fail::Group);
    };
    if pile.attach(&tip_hole, HoleDir::Pull).is_err() {
        return Err(super::fail::Fail::Group);
    }

    let mut tree = ocall::tree();
    let mut desk = desk();
    // **装配期**：还没有协调门牌（装配者那一帧到了才是 `Some`）⇒ 门禁放行——**装配期不在门禁
    // 这条轴上**（理由见 [`may`]），不是给它的例外。
    let mut session: Option<Session> = None;
    // 协调那一帧递来的两格号（名册 / 盟册，各自那一域自己把门牌交过来）。**两帧、次序不定**。
    let mut coord = Coord::default();
    // **那本账**：一格一条，两轴都记（见 [`Book`]）。
    //
    // "活着"那一问与树收的是**同一枚函数指针**（`ocall::vested_by`）——账要问的"主人还在吗"
    // 与树要问的"这一枚还答得出吗"是同一句话，故不另开一个 trait。
    let mut book: Book = Ledger::new(ocall::vested_by);
    // 收帧的那一页：**在循环外备一次**——门的缓冲不再是"这一族最大的那一帧"（`REQ_LEN`），
    // 而是**载体的一页**：界判在 `Push`，故客人推得进来的最长就是一页；拿家族帧当缓冲时，
    // 比它长的那一枚（≤ 一页）客人推得进、这道门取不出 ⇒ 那一位客人从此没人招待。
    // **本处是 A 那一刀漏掉的第六处**（前五处：principal / coalition / router / rtc / 板），
    // 由 `probe-bound` 那一条探针当场抓出来（读数见 `harness/src/probe_bound.rs`）。
    let mut buf: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    if buf.try_reserve_exact(PAGE_SIZE).is_err() {
        return Err(super::fail::Fail::Room);
    }
    buf.resize(PAGE_SIZE, 0);
    loop {
        // 一、补齐两件事（收提示 + 认领答话路、认出问话孔并挂组）。
        let settling = settle(&mut desk, &pile, &tip_hole, &mut coord, &mut session);
        // 二、等一格有事。**一个等待**：提示孔或任意一位客人的问话孔。
        let millis = if settling { Wait::AtMost(SETTLE_MS) } else { Wait::Forever };
        let Ok(Some((tok, _dir))) = pile.await_(millis) else {
            let _ = desk.sweep();
            continue;
        };
        // 提示孔那一格由下一轮的 `settle` 收（它非阻塞地拉）；这里只管"是哪位客人的问话孔"。
        if tok != tip
            && let Some(guest) = desk.guest(tok).copied()
        {
            serve_one(&mut tree, guest, session.as_ref(), &mut book, &mut buf);
        }
        // 三、**看出来的**那一档：那一枚答不出 ⇒ 剔格子（没有"他说走了"那一档）。
        let _ = desk.sweep();
    }
}

/// 补齐两件事，返"还有没有没补齐的"。
///
/// - **提示**：装配者推来的客人号（一个号，8 字节）。**非阻塞地拉**——必须在这里拉，
///   不能只在"组唤醒"那一支拉：装配者的推**可能早于本线程把提示孔挂进组**（那一条推
///   落在一个还没有转发登记的站点上），醒不来就得靠这一拉吃到它；
/// - **协调那一帧**（16 字节）：装配者把"**哪一位域** + **它是哪一双眼睛**"直接递过来
///   （见 [`ocall::CoordFrame`]）。两帧、次序不定：名册那一
///   枚到了才开闸（门禁从此判得了身份），盟册那一枚到了 [`Rule::In`] 才判得了。
///   **长度即语义**：8 = 一位客人，16 = 这一帧；
/// - **答话路**：装配者转授来的那一枚 ⇒ `admit` 收一位客人；
/// - **问话孔**：客人**自己**交来的那一枚 ⇒ 认出来就 `arm` + 挂进组。
fn settle(
    desk: &mut Desk,
    pile: &Pile,
    tip: &mail::HolePie,
    coord: &mut Coord,
    session: &mut Option<Session>,
) -> bool {
    // 提示：拉干净（单槽，一位客人一条）。**非阻塞**——它的到达是别人在做的事。
    // 缓冲按**最大的那一帧**备（16），故协调那一帧也吃得下——小缓冲会把长帧读成"读不懂"。
    let mut frame = [0u8; ocall::CoordFrame::LEN];
    let mut pending = false;
    loop {
        let Ok(n) = tip.pull_timeout(&mut frame, Wait::POLL) else {
            break;
        };
        if n == ocall::CoordFrame::LEN {
            // **开闸**：两格——哪一位域、它是哪一双眼睛。各自那一枚门牌由那一域**自己**交进来
            // （装配者只递号）；从这里往后，门外那一问（[`gate`](protocol::system::operator::core::gate)）
            // 判得了身份。
            //
            // 后 8 字节能不能翻（表外的眼睛码）归 [`Eyes`] 自己的 `Field::fetch`：读不懂 ⇒
            // **报一句，别静默**——门禁会一直判不了，而"为什么"要看得见。
            let Some(rec) = ocall::CoordFrame::fetch(&frame[..n]) else {
                say("operator: coord role unknown");
                continue;
            };
            match rec.eyes {
                Eyes::Roster => coord.roster = Some(rec.who),
                Eyes::League => coord.league = Some(rec.who),
            }
            // 两枚各自到了就各自重建一次（幂等）：名册先到 ⇒ 门禁立刻能判；盟册后到 ⇒ 补上
            // `Rule::In`。名册认不出（记号/开者对不上）要**报一句**，别静默——门禁会一直答
            // "判不了"。
            let renewed = Session::of(*coord);
            if renewed.is_none() {
                say("operator: coord not recognised");
            }
            *session = renewed;
            continue;
        }
        // 短的那一帧：**一位客人**（8 字节，[`TaskId`]，见 `operator::bridge::tell`）。
        let Some(client) = TaskId::fetch(&frame[..n]) else {
            continue;
        };
        match reply_of(client) {
            Some(reply) => match desk.admit(client, reply) {
                // 收了。
                Ok(_) => {}
                // **重放**（提示是单槽，可能重放）：一位客人只占一格，旧的那一格**原样留着**
                // ——这一趟不动账，也不打行（重放是常态）。交来的那一枚与账上那一枚**是同一枚**
                // （按"开者 + 树路记号"认，一位客人只 seat 一次），故这里不丢任何东西
                // （见 `DeskFail::Already` 的照实记）。
                Err(DeskFail::Already) => {}
                // **满了**：这位客人进不来，而**它自己不知道**——它的问话孔没人管，第二次
                // 问话会堵在单槽上（整台机器收不了场）。故这一格**报一句，别静默丢一位客人**；
                // 这本账的上限史（撞满过三次）见 `desk.rs` 里 `Desk` 那一格的照实记——这一格
                // 就是它量出来的那一次。
                Err(DeskFail::Full) => say("operator: desk full"),
            },
            // 次序被破坏（提示先到、答话路不在本表里）：报一句；客人那边会报它自己的超时。
            None => say("operator: no reply"),
        }
    }
    // 还没挂上问话孔的那几格：**账自己按格子号走一遍**（见 [`Desk::arm_pending`]）——
    // 调用方这一侧因此既不必按常数开数组（"一本账的容量渗到别人的栈上"那一格），
    // 也不必为"抄一份"再分配一次（`collect` 那次分配没有预留，失败同样是 abort）。
    pending |= desk.arm_pending(
        |who| ask_of(who),
        |ask| {
            pile.attach(&mail::HolePie::from_token(ask), HoleDir::Pull)
                .is_ok()
        },
    );
    pending
}

/// **那本账**：一格一条，记着两轴（谁许用 / 归谁改）。
///
/// 正文在协议那一侧（[`protocol::system::operator::core::ledger`]），本域只做三件事：**接上"活着"那一问**
/// （`ocall::vested_by`，与树收的是同一枚函数指针）、**接上"那一格还是不是那一格"那一问**
/// （[`fresh`]，一趟读）、**按钥匙查**。
///
/// **照实记（这一格原来住在这里）**：前身是 `type Publishers = Vec<(Where, Name, TaskId,
/// PieToken)>`，只记"声明归自己"的那些。它在这份文件里量错过两次——第一版按**号**记，而
/// `land` 那一问手里只有坐标，于是**整道判据被跳过**，`probe-owner` 当场顶掉了 `/device/uart`
/// 的牌子（读数 `land=OK id=5`）；改按坐标记之后，`find` / `trim` 手里又只有号，于是要补一趟
/// `container_of`（O(树) 的全树递归）。
///
/// 两条寻址打的是同一格——**这一条现在由结构说话**（一条记录、两把钥匙），不再靠两处各补
/// 一次补丁。而它搬进「约」之后进过宿主靶（**那台靶已删**）：**没有门的档 = 没有编译过的档**。
type Book = Ledger<Id, Id>;

/// 招待一位客人：从**它的问话孔**读一帧、交给树、把答话推进**它的答话路**。
///
/// 组已经说了"这一枚有话"，故这一读读得动；期限给 `0` 是**再确认**，不是轮询。
///
/// `buf` = **调用方那一页**（`serve` 在循环外备一次）：收帧不在这里分配，也不是家族帧那么大
/// ——客人的最长帧由载体定（一页），门就得有一页才接得住（见 `serve` 那一格的照实记）。
fn serve_one(
    tree: &mut Operator,
    guest: Guest,
    session: Option<&Session>,
    book: &mut Book,
    buf: &mut [u8],
) {
    let Some(ask) = guest.ask() else {
        return;
    };
    // **收帧用调用方那一页**（[`Slip::land`]）：比家族最长那一枚更长的一条也取得出来、
    // 解得失败 ⇒ 照旧答一句 `BAD`，而槽也空了。拿家族帧那么大的一只缓冲收不下它，而核**不丢**
    // 取不出的消息（见 `Slip::land` 的照实记与 `probe-bound` 那两趟——这一门正是那一格
    // 从前量过的地方）。
    // 收：**两格失败在这一门同一落点**（`answer` 收的还是 `Option`：读不懂与期限到了都答 `BAD`）。
    let decoded = Slip::<ocall::Req<'_>>::seal(ask).land(buf, Wait::POLL).ok();
    let said = answer(tree, decoded, guest.who(), session, book);
    // 答一句：**形状由 [`ocall::Union`] 说**——装与发都不在这一层写字节（那四种答形在线上分不开，
    // 故读的那一面由问的人认，见 [`ocall::Said`] 的照实记）。
    // `.ok()`：装不上那一格按构造到不了（`Buf` 由本族 `Message` 自己给，见 `Slip::load`
    // 的照实记）；真到了那里，这一答就发不出去。
    let _ = Slip::<ocall::Union>::seal(guest.reply())
        .load(said)
        .ok()
        .map(|s| s.ship());
}

/// 把一句问交给树，编出一句答（**答话有四种形状**，见 [`ocall`] 的帧那一节）。
///
/// **形状由 [`ocall::Wire`] 说**（收帧那一侧已经按动作解好了），**答由 [`ocall::Union`] 说**：
/// 解不出来就是一句读不懂的帧（不猜、不崩）；`land` 那一码**必须带入口号**（没带同样解不出来）。
fn answer(
    tree: &mut Operator,
    ask: Option<ocall::Wire>,
    who: TaskId,
    session: Option<&Session>,
    book: &mut Book,
) -> ocall::Union {
    // 空帧 / 长度不对 / 表外的动作码：读不懂（答 `BAD`）。
    let Some(ask) = ask else {
        return ocall::Union::Status(ocall::BAD);
    };
    // 路太长：**先按上限挡掉**，别把一条被截断的路当成真的（核心那几条原语也各有这条判据）。
    if let ocall::Wire::Road(_, count) = ask {
        if count > Operator::ROAD_MAX {
            return ocall::Union::Status(ocall::FULL);
        }
    }
    // **门外那一问**：两条会**交出权柄 / 毁掉别人那一格**的原语先过门禁——`find`（把那一枚
    // 授出去）与 `trim`（把别人的名字剪掉）。`land` **不在这里**判：它是"改我自己那一格"，
    // 它的准入是**那一格自己的规矩**（见下面的两支）。四条只读结构的
    // （`part` / `list` / `seek` / `name`）一律不判。
    //
    // **两轴分家**（这一刀的新内容）：
    //
    // - **用**那一轴（谁许用这一格）住在那一格的账上，由 [`Book::rule`] 答；`find` 判它；
    // - **改**那一轴（谁许改这一格）同样住在账上，由 [`Book::claimable`] 答；`land` / `trim` 判它；
    // - 两轴都**不**在树里（树至今不知道"规矩"这个词），也**不**在核心（核心是同步纯函数，
    //   发不出那两条问身份的消息）。
    match ask {
        // **`find` 看这一格自己的"用"那一轴**。
        //
        // 照实记（为什么这里从"全局默认"变成"逐格规矩"，而不是反过来）：上一刀把主人判据也
        // 挂在 `find` 上，`uart` 声明归自己之后，`echo` 当场取不到 `/device/uart`——机器还在，
        // 控制台没人读（`examine` 0/3）。**读是公开的，写才归属主**：改那一轴（`land` 那一格
        // 的 `mine`，账里记成 `Owner`）管的是**改这一格**，不是**用这一格**。这一刀让「用」有
        // 自己的那一格，故默认值（公开）与主人那
        // 一轴不再互相牵制。
        ocall::Wire::Find(id) => {
            let rule = book.rule(Key::Id(id), |id| fresh(tree, id));
            let ruling = may(tree, session, who, rule);
            if !ruling.passed() {
                return ocall::Union::Status(ruling.wire());
            }
        }
        ocall::Wire::Trim(id) => {
            if !book.claimable(Key::Id(id), who, |id| fresh(tree, id)) {
                return ocall::Union::Status(ocall::DENIED);
            }
            let ruling = may(tree, session, who, Rule::Public);
            if !ruling.passed() {
                return ocall::Union::Status(ruling.wire());
            }
        }
        // **`land` 也要先问身份**（与 `find`/`trim` 同一道门）：它虽然不动别人的格子，
        // 但"往树上挂东西"这件事本身要求来的人是个**已绑身份**——否则没身份的任务就能
        // 往命名空间里塞条目。**实测栽过一次**：漏了这一支，负证客人当场落牌成功
        // （读数 `probe: tree land=OK id=7`）。
        ocall::Wire::Land { .. } => {
            let ruling = may(tree, session, who, Rule::Public);
            if !ruling.passed() {
                return ocall::Union::Status(ruling.wire());
            }
        }
        _ => {}
    }
    let said = match ask {
        // **两条答号的**：立/分的人自己得知道立成了几号——答案体不是一格状态。
        ocall::Wire::Land {
            at,
            name,
            entry,
            rule,
            mine,
        } => {
            // **"改这一格"那一轴**：落之前先看这一格现在归谁——不是我就拒。占了的位置由
            // **活着的主人**说了算；空着的位置谁都能落，落了就登记成他的。
            //
            // **按坐标查**（不是按号）：`land` 那一问发生在动树之前，而 `land` 换绑**不动号**
            // ——故那一刻手里只有坐标。见 [`Book`] 那段照实记（第一版按号查，真机上把这一道
            // 判据整个跳过去了）。
            if !book.claimable(Key::At(at, name), who, |id| fresh(tree, id)) {
                return ocall::Union::Status(ocall::DENIED);
            }
            // **一问一动**：要位 → 落树 → 记账全在 [`Ledger::land`] 里，漏不掉中间那一步。
            // 账上记的那一枚是**落树那一刻挂上去的**（`entry`）：`mine = false` 是**放弃归属**
            // ——与"改规矩"同走这一条（两条没有独立入口：这一问是**整值赋值**，
            // 见 [`protocol::system::operator`] 那一节）。
            return match book.land(at, name, entry, rule, mine, who, || {
                tree.land(at, name, entry)
            }) {
                Ok(id) => ocall::Union::Entry(id),
                Err(fail) => ocall::Union::Status(ocall::fail_to_code(Some(fail))),
            };
        }
        ocall::Wire::Part { at, name } => {
            return match tree.part(at, name) {
                Ok(id) => {
                    // **那个窄口子**：`part` 碰到一枚 `Tile` 会静默把它顶成一块 `Pane`
                    // ——那一格已经不是"放 Pie 的那一格"了，故账上那一行要销掉。
                    // （漏了也不会答错：`fresh` 那一次对真相兜着；这只是不让账留一条陈的。）
                    book.drop(id);
                    ocall::Union::Entry(id)
                }
                Err(fail) => ocall::Union::Status(ocall::fail_to_code(Some(fail))),
            };
        }
        // 查到就**把树上那一份转授给客人**：Pie 本身不从报文里走，从会话里走；而
        // **它在客人表里的号**从这条答话里走（[`ocall::Union::Seed`]）——客人拿它一次 `Reserve`
        // 就认得出，不必扫自己的表（见 [`ocall::Union::Seed`] 的照实记）。
        // "查不到"与"授不出去"是两件事，故查的结论优先：`said` 先答，其次才轮到 `grant`。
        ocall::Wire::Find(id) => {
            let mut seed = None;
            let mut grant = Ok(());
            let said = tree.find(id, |pie| {
                grant = ocall::ship(pie, who).map(|at| seed = Some(at));
            });
            if said == Err(Fail::Dead) {
                // 核心**已经**把那一格剔了（"惰性剔死"）——顺手销账，别留一条陈的。
                book.drop(id);
            }
            // 三步都成 ⇒ 答 `[OK][那一格]`；任何一步没成 ⇒ 照旧一格状态（不猜）。
            let fail = said.err().or(grant.err());
            return match (fail, seed) {
                (None, Some(seed)) => ocall::Union::Seed(seed),
                (Some(fail), _) => ocall::Union::Status(ocall::fail_to_code(Some(fail))),
                // 授成功却没拿到号：这是内核契约破了（`ship` 的成功值就是那一格），不猜。
                (None, None) => ocall::Union::Status(ocall::BAD),
            };
        }
        ocall::Wire::Trim(id) => {
            let said = tree.trim(id);
            if said.is_ok() {
                // 格子从树上没了 ⇒ 那一行也走。
                book.drop(id);
            }
            said
        }
        // **三条答数据的**：答案体不是一格状态，故各自编各自的帧（成败都在帧里）。
        ocall::Wire::List(at) => {
            return match tree.list(at) {
                Ok(ids) => ocall::Union::List(Listing::of(ids)),
                Err(fail) => ocall::Union::Status(ocall::fail_to_code(Some(fail))),
            };
        }
        ocall::Wire::Name(id) => {
            return match tree.name(id) {
                Ok(name) => ocall::Union::Name(name),
                Err(fail) => ocall::Union::Status(ocall::fail_to_code(Some(fail))),
            };
        }
        // **译号那一档**：名字只能走到这里——拿到号之后，其余原语一律按号走。
        ocall::Wire::Road(road, count) => {
            return match tree.seek(&road[..count.min(Operator::ROAD_MAX)]) {
                Ok(id) => ocall::Union::Entry(id),
                Err(fail) => ocall::Union::Status(ocall::fail_to_code(Some(fail))),
            };
        }
    };
    ocall::Union::Status(ocall::fail_to_code(said.err()))
}

/// 转授来的那一枚答话路（**写端**，落在本表里）。
///
/// 两格判据，都是确定的号：
///
/// - `owner == who` —— **谁的**：那扇门是**这位客人**开的（副本共享同一事实）；
/// - **记号 == `operator`** —— 那一枚是**树路**上的一枚。
///
/// **照实记（分开之后的改动）**：原来还有第三格 `vestor == 装配者`——"谁转的"。它随
/// **装配者是谁**而失效：树这一侧现在有两个域跟它打交道（引导域起它、编排域接客人），
/// 而它认的本来就是"**这扇门是谁开的**、**走的哪条路**"，不是"谁转的"。次序那件事仍由
/// `settle` 管（答话路没到就先报一句，见那里）。
fn reply_of(who: TaskId) -> Option<PieToken> {
    // 多枚不可能（`seat` 的同名判据兜着）⇒ 不说。
    claim(Mark::of(LINK), who, None)
}

/// 这一位客人**自己**交来的那一枚问话孔。
///
/// 判据两格，缺一不可：`owner == who`（那扇门是它开的）**且** 记号 == `ask`（它亲手铸的
/// 那一枚）——客人交来的**入口**也满足前两格（都是它铸、它交的），两件事只有记号分得开。
fn ask_of(who: TaskId) -> Option<PieToken> {
    // 多枚**是契约被破**（一个域只该铸一枚问话孔）⇒ 说一句。
    claim(ASK_MARK, who, Some("operator: two asks"))
}

/// **认领恰好一枚**：按「谁开的 + 记号」扫全表，答**第一枚**。
///
/// 三处认领（答话路 / 问话孔 / 门牌）原先各写一遍同一段扫表，且**都取第一枚而从不看有几枚**。
/// 这一格把那段扫表合成一处；`more` 是"多枚要不要说一句"。
///
/// # 照实记：这一格原来**一律"多枚就报一句"**，真机上每启一次误报几十行
///
/// 理由是当时以为"命中两枚 = 纪律被破"。**真机把它顶掉了**：`land` 自己那一手也会把入口
/// 那一枚交到持树者表里（树就是这么存东西的），而门禁那一手又交一枚——**凡在树上挂了自己
/// 入口的域，持树者手里必然有两枚 `ENTRY` 记号的副本**，两枚的 `opened_by` 都是那个域
/// （副本共享同一事实）。实测：一次启动 `operator: two entries` 出了 **60 行**。
///
/// 这个过滤器**分不出副本与"第二扇门"，也不该分**。故：
///
/// | 处 | 记号 | 多枚是 |
/// |---|---|---|
/// | [`reply_of`] | `LINK` | **不可能**——`Quay::seat` 的同名判据兜着（"同一位、同一记号只可能有一枚"） |
/// | [`find_face`] | `ENTRY` | **结构性正常**（上面那一笔）⇒ 不说 |
/// | [`ask_of`] | `ASK` | **契约被破**：一个域只该铸一枚问话孔（裸 `unseal_hole`，没有同名闸），多出来的那枚永远没人读它的推 ⇒ 说一句 |
///
/// 而"取第一枚"在三处都正当：命中的几枚背后是**同一扇门**（同一份 `HoleMeta`），任一枚都通。
///
/// 还有两格记着（不在这一刀里）：
///
/// - **别把它做成 fail-closed**：两枚孔的出现与持树者查表之间有**天然竞态**（持树者每 1ms 查
///   一次，而两枚孔之间只隔两个 envcalls）⇒ "拒"是间歇的，且那位客人从此没人给它挂孔
///   （持树者会永远停在"还有人没挂上"那一档）；
/// - **干净的关法**是让 `ask_hole` 与入口那一枚也走**有名有姓的泊位**（`Quay::seat` 那条路
///   已有同名闸），把"只可能有一枚"从纪律变成**构造**——那是客侧形状的改动，另一刀。
/// - **照实记（裁定甲之后松掉的一格）**："本表读不动 ⇒ 一枚都不认"那条 fail-closed 随
///   `mail::collect` 的 `Err` 通道一起没了：迭代器读到读不动就**打住**，与"扫完了"合流
///   ⇒ 读不动之前已经认到的那一枚**照旧交出去**。今天够不着（那条 `Err` 是防御读数），
///   但它是**松开**，不是等价。
fn claim(mark: Mark, who: TaskId, more: Option<&str>) -> Option<PieToken> {
    let mut hits = mail::pies().filter(|p| p.owner == who && p.mark == mark);
    let first = hits.next()?;
    // **第二枚 ⇒ "只可能有一枚"那条纪律破了**：说话（`more` 那一格就是这句话）。
    if hits.next().is_some() {
        if let Some(note) = more {
            say(note);
        }
    }
    Some(first.token)
}

/// 持树者的读数：**只在出岔子时说话**（正常一轮什么都不打）。
fn say(msg: &str) {
    let _ = runtime::env::debug::put(msg);
}
