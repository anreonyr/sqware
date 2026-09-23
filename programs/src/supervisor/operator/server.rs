//! operator::server — **持树者**：自己的域里的一枚线程守着那棵树（一枚线程 + 一个组，无轮询）
//!
//! 三侧分家之后本文件只放**持树者**：自己的域里的一枚线程守着那棵树（一枚线程 + 一个组，无轮询）；两侧共用的图与说明见 [`super`] 的"载体"那一节，
//! 帧与记号见 [`protocol::operator::call`]。

use env::{HoleDir, Mark, Name, PieToken, TaskId};
use runtime::core::port::{self, Access, Policy};
use runtime::core::tole::Tole;
use runtime::env::mail;
use runtime::env::room::exit_with;
use runtime::env::unit as utask;

use protocol::operator::call as ocall;
use protocol::operator::gate::{Code, Control, verdict};
pub use protocol::operator::{ASK_MARK, LINK, TIP_MARK};
use protocol::operator::{EntryId, Fail, Operator, Rule, Where};
use protocol::system::board::call as bcall;

use alloc::vec::Vec;
use protocol::principal::client::Face as PolicyFace;
use protocol::principal::core::PrincipalId;

use super::desk::{Desk, Guest, desk};

/// 协调那一帧的长度（**长度即语义**：8 = 一位客人，16 = 这一帧）。
///
/// 与 `programs/src/supervisor/operator/bridge.rs` 的同一格必须同值——那边是**推**这一侧。
const COORD_FRAME: usize = 16;

/// **这一刀先按"公开"判**：条目上还没有逐格规则（那是下一刀），故所有条目共用这一条
/// 默认值。它的准确含义是"**任何已绑身份都可以**"——没绑的仍然被拒（[`Rule::Public`] 那一格）。
///
/// 默认必须是它，否则既有的 11 条 `tree part=0 … land=0 find=0 got=true` 会当场塌：
/// 树自己那条提示、principal 补绑之后的每一条服务，全都是已绑身份。
const DEFAULT_RULE: Rule<u32, u32> = Rule::Public;

/// 还在"补齐两本账"（答话路未认领 / 问话孔未挂上）时，一轮等多久（毫秒）。
///
/// **不是轮询**：账补齐之后这一等就变成 `usize::MAX`（由组唤醒）；这个短期限只在装配窗口
/// 里用——那几步的到达是**别人**在做（装配者转授、客人自己交孔）。
const SETTLE_MS: usize = 1;

/// 持树者起不来时的编号（指"死在头几步的哪一步"）。
const E_SIRE: usize = 1;
const E_TIP: usize = 2;
const E_GROUP: usize = 3;

// ── 门外那一问（门禁）────────────────────────────────────────

/// **协调那一份**：身份服务那一枚门牌（装配者转授进来的，见 [`COORD_FRAME`]）。
///
/// 树**不当自己的客人**：它不去 `seek("/sys/principal")`，而是由装配者直接给。理由见
/// `programs/src/supervisor/operator/bridge.rs` 的 `COORD` 那段照实记（自指 ⇒ 环）。
struct Session {
    face: PolicyFace,
}

impl Session {
    /// 认出协调那一枚门牌：**按"谁开的 + 记号"两格**在本表里找（协调那一帧只带号）。
    ///
    /// 两格都是确定的：那扇门是**身份服务**开的（副本共享同一事实），记号 = 服务入口记号
    /// （`bcall::ENTRY_MARK`）。**不必装配者转授**——身份服务自己在 `serve_tree` 之后把它
    /// 直接交给持树者（见 `operator/bridge.rs` 的 `COORD` 照实记）。
    fn of(who: TaskId) -> Option<Session> {
        let mark = bcall::ENTRY_MARK;
        let mut index = 0usize;
        loop {
            let (token, _perm, _vestor) = mail::collect(index).ok()?;
            // 越界哨兵：这一遍扫完了。
            if token.get() == 0 {
                return None;
            }
            index += 1;
            if ocall::opened_by(token) == Some(who) && ocall::marked_as(token) == Some(mark) {
                let face = PolicyFace::of(token).ok()?;
                return Some(Session { face });
            }
        }
    }
}

/// 门禁要的两个事实都从这一份出：问身份（`resolve`）与谓词（`heir`）。
///
/// **一面门牌在手就够**：`amid` 那一格今天没人问（[`DEFAULT_RULE`] 是 `Public`），故这一刀
/// 不接结盟那一枚；要接时把它的门牌也递进来、[`Control::amid`] 转发一句即可。
impl Control for Session {
    fn who(&self, tid: TaskId) -> Result<Option<u32>, ()> {
        match self.face.resolve(tid, MS) {
            Ok(found) => Ok(found.map(|p| p.get() as u32)),
            Err(_) => Err(()),
        }
    }

    fn heir(&self, a: u32, b: u32) -> Result<bool, ()> {
        self.face
            .heir(
                PrincipalId::new(a as usize),
                PrincipalId::new(b as usize),
                MS,
            )
            .map_err(|_| ())
    }

    fn amid(&self, _: u32) -> Result<bool, ()> {
        // 这一刀没有 `Rule::In`（见上）：答"问不到"，而不是答"否"——**判不了**与
        // "不在那枚盟里"是两件事，后者会让客人当场放弃。
        Err(())
    }
}

/// **门禁的入口**：`session` 为 `None` = **装配期**（树手里还没有门牌）⇒ 放行；`Some` =
/// 按 [`DEFAULT_RULE`] 判。
///
/// 装配期放行是**定义**不是例外：树接手时（principal 挂 `/sys/principal`、coalition 挂
/// `/sys/coalition`）整个装配都还没走完，门禁无从判起；而那两条路的来路是装配者**直接铺的**
/// （他发起的 `Ship`），不是"从问话孔进来的客人请求"。
fn may(session: Option<&Session>, who: TaskId) -> Code {
    match session {
        None => Code::Ok,
        Some(s) => match verdict(s, who, DEFAULT_RULE) {
            // 「手里没有门牌」在客人那一侧与「判不了」同一格（都可重试）；`Some` 的时候
            // 不该出现它，真出现了也按"判不了"走，不按"放行"。
            Code::Blind => Code::Unjudged,
            other => other,
        },
    }
}

/// 问身份那两条边要用的期限（毫秒）。**必须有界**：协调服务不在时不能把树挂死。
const MS: usize = 1000;

// ── 持树者侧（本域的服务线程）────────────────────────────────

/// 起服务：**铸提示孔交给装配者，然后一枚线程招待所有客人**。
///
/// 头两步是契约：装配者按 `(本域, tip)` 两格认领提示孔（[`attach`] 的 `host_of`），
/// 而提示一到它就认为"答话路必已在本表里"（转授在前、提示在后）。
pub fn serve() -> ! {
    let Ok(assembler) = utask::sire() else {
        say("operator: no sire");
        exit_with(E_SIRE);
    };
    // 提示孔：本线程铸的那一枚（客人号从这里进来），副本交给生我者。**记号 = `tip`**。
    let Ok(tip) = mail::unseal_hole(TIP_MARK) else {
        say("operator: no tip");
        exit_with(E_TIP);
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
        say("operator: tip not handed");
        exit_with(E_TIP);
    }
    // **一个组**：提示孔 + 每位客人的问话孔。提示孔也挂进来，故"来客人了"与"有人问话"
    // 是**同一个等待**。本线程独享它（`shared = false`）。
    let Ok(tole) = Tole::unseal(false) else {
        say("operator: no group");
        exit_with(E_GROUP);
    };
    if tole.attach(&tip_hole, HoleDir::Pull).is_err() {
        say("operator: tip not hung");
        exit_with(E_GROUP);
    }

    let mut tree = ocall::tree();
    let mut desk = desk();
    // **装配期**：还没有协调门牌（装配者那一帧到了才是 `Some`）⇒ 门禁放行，见 [`may`]。
    let mut session: Option<Session> = None;
    // **规矩那份账**：只记"归落牌那一位"的条目（见 [`Publishers`]）。
    let mut publishers: Publishers = Vec::new();
    loop {
        // 一、补齐两件事（收提示 + 认领答话路、认出问话孔并挂组）。
        let settling = settle(&mut desk, &tole, &tip_hole, &mut session);
        // 二、等一格有事。**一个等待**：提示孔或任意一位客人的问话孔。
        let millis = if settling { SETTLE_MS } else { usize::MAX };
        let Ok(Some((tok, _dir))) = tole.await_(millis) else {
            let _ = desk.sweep();
            continue;
        };
        // 提示孔那一格由下一轮的 `settle` 收（它非阻塞地拉）；这里只管"是哪位客人的问话孔"。
        if tok != tip
            && let Some(guest) = desk.guest(tok).copied()
        {
            serve_one(&mut tree, guest, session.as_ref(), &mut publishers);
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
/// - **协调那一帧**（16 字节）：装配者把"身份服务是谁 + 它那枚门牌在本表里的号"直接递过来
///   （见 `programs/src/supervisor/operator/bridge.rs` 的 [`COORD`]）。收到它就**开闸**——
///   门禁从此判得了身份。**长度即语义**：8 = 一位客人，16 = 这一帧；
/// - **答话路**：装配者转授来的那一枚 ⇒ `admit` 收一位客人；
/// - **问话孔**：客人**自己**交来的那一枚 ⇒ 认出来就 `arm` + 挂进组。
fn settle(
    desk: &mut Desk,
    tole: &Tole,
    tip: &mail::HolePie,
    session: &mut Option<Session>,
) -> bool {
    // 提示：拉干净（单槽，一位客人一条）。**非阻塞**——它的到达是别人在做的事。
    // 缓冲按**最大的那一帧**备（16），故协调那一帧也吃得下——小缓冲会把长帧读成"读不懂"。
    let mut frame = [0u8; COORD_FRAME];
    let mut pending = false;
    loop {
        let Ok(n) = tip.pull_timeout(&mut frame, 0) else {
            break;
        };
        if n == COORD_FRAME {
            // **开闸**：两格号——身份服务是谁、它那枚门牌种在本表里的哪一号。
            // 从这里往后，门外那一问（[`gate`](protocol::operator::gate)）判得了身份。
            let who =
                TaskId::new(u64::from_le_bytes(frame[..8].try_into().unwrap_or([0; 8])) as usize);
            *session = Session::of(who);
            if session.is_none() {
                // 门牌认不出（记号/开者对不上）：**报一句**，别静默——门禁会一直答"判不了"。
                say("operator: coord not recognised");
            }
            continue;
        }
        let Some(id) = frame.get(..8) else {
            continue;
        };
        let client = TaskId::new(u64::from_le_bytes(id.try_into().unwrap_or([0; 8])) as usize);
        match reply_of(client) {
            Some(reply) => match desk.admit(client, reply) {
                // 收了。
                Ok(_) => {}
                // **重放**（提示是单槽，可能重放）：一位客人只占一格，无事。
                Err(Fail::NonEmpty) => {}
                // **满了**：这位客人进不来，而**它自己不知道**——它的问话孔没人管，第二次
                // 问话会堵在单槽上（整台机器收不了场）。故这一格**报一句，别静默丢一位客人**；
                // 格数见 `Desk::CAP` 那条照实记（这一格就是它量出来的那一次）。
                Err(_) => say("operator: desk full"),
            },
            // 次序被破坏（提示先到、答话路不在本表里）：报一句；客人那边会报它自己的超时。
            None => say("operator: no reply"),
        }
    }
    // 先抄一份"还没挂上的"：`unarmed` 借住这本账，而下面要改它。
    // **先抄一份"还没挂上的"**：`unarmed` 借住这本账，而下面要改它。这里**不按常数开数组**
    // （那正是"一本账的容量渗到别人的栈上"那一格）：直接 iterate 出来逐条处理即可。
    let waiting: alloc::vec::Vec<(usize, TaskId)> = desk.unarmed().collect();
    for &(slot, who) in &waiting {
        match ask_of(who) {
            Some(ask) => {
                let hung = desk.arm(slot, ask).is_ok()
                    && tole
                        .attach(&mail::HolePie::from_token(ask), HoleDir::Pull)
                        .is_ok();
                if !hung {
                    let _ = desk.unarm(slot);
                    pending = true;
                }
            }
            None => pending = true,
        }
    }
    pending
}

/// **谁落的这一格**：`(那一格的坐标, 落牌的 TID, 那一刻挂上去的那一枚 Pie)`。**只登记 `Rule::Owner` 的那些**——默认
/// （公开）的条目一个字节都不占，既有装配读数因此一字不改。
///
/// **按坐标记，不按号记**（照实记：第一版按号记，真机上当场量错）。两个理由：
///
/// 1. **号只在树里认得出**，而门口那一问发生在**动树之前**——要么先查一次（多一趟），
///    要么拿到的号与那一格对不上；
/// 2. **换绑不动号**（`land` 那条规矩），所以号也分不出"这一格换过人"。
///
/// 坐标（`Where` + 名）在 `land` 那一帧里**本来就有**，比对是 O(账长)。这份账是**服务侧的**，
/// 不是树的一份数据：树的核心（`crates/protocol/src/operator/core.rs`）至今不知道"规矩"这个词。
/// 代价照实记：**持树者重启这条账就没了**（而条目还在）⇒ 那些格回落到"公开"。今天没有重启
/// （退出即关机），故这是记着的一笔。另一笔：**落牌那一方退了之后这个主人就永远不在场**，
/// 那一格从此顶不掉——见 `docs/operator-gate.md` 的已知边界。
type Publishers = Vec<(Where, Name, TaskId, PieToken)>;

/// 那一格的主人（`None` = 这一格没被声明过归属）。
fn publisher_of(publishers: &Publishers, at: Where, name: Name) -> Option<(TaskId, PieToken)> {
    publishers
        .iter()
        .find(|(slot_at, slot_name, _, _)| *slot_at == at && *slot_name == name)
        .map(|(_, _, who, pie)| (*who, *pie))
}

/// **那一格的主人还在不在场**。
///
/// 问的是"挂上去的那一枚还答得出吗"——**与 `VestedBy` 同一句话**（`Reserve` 那一问）：
/// 落牌那一方退场时，内核的退场钩子把它开的那些资源**封印**（`gate::doom` 的
/// `seal_owned`）⇒ 这一枚从此答 `None`。故不需要任何看门狗/通知，一次查询就有答案。
///
/// **答不出来（表里没这一枚 / 已不在）也按"不在场"算**：那一格的主人已经不可能再
/// 管它了——见 [`claim`] 那条规矩。反过来，**没有这一格**（`publisher_of` 返回 `None`）
/// 是另一件事：那是"没声明过归属"，不是"主人没了"。
fn present(pie: PieToken) -> bool {
    ocall::vested_by(pie).is_some()
}

/// 登记/改登记那一格的主人（换绑之后主人跟着换——**能换的人**由上一道判据管）。
fn remember(publishers: &mut Publishers, at: Where, name: Name, who: TaskId, pie: PieToken) {
    match publishers
        .iter_mut()
        .find(|(slot_at, slot_name, _, _)| *slot_at == at && *slot_name == name)
    {
        Some(slot) => {
            slot.2 = who;
            slot.3 = pie;
        }
        None => {
            if publishers.try_reserve(1).is_ok() {
                publishers.push((at, name, who, pie));
            }
        }
    }
}

/// 那一格**此刻归不归 `who` 管**：是主人本人、或那一格的主人已经不在场（⇒ 谁都能接手）。
///
/// 这条规矩是"规矩属于**活着的**主人"的确切含义。**照实记（为什么需要它）**：落牌那一方
/// 退场之后，它声明归自己的那一格**永远顶不掉**——主人不在场，没人能换绑。今天 `uart`
/// 不退场也没有重启，所以这只是一笔记着的账；但只要有一台**会死的**服务敢声明归属，
/// 它一死就会在命名空间里留一块**没人能改的墓碑**。
///
/// **接手是"看出来的"，不是"被通知的"**——与树的惰性剔死（`find` 路上剔）同一条形状：
/// 不问就不动，问了才发现主人没了。
fn claimable(publishers: &Publishers, at: Where, name: Name, who: TaskId) -> bool {
    match publisher_of(publishers, at, name) {
        None => true,                             // 没声明过归属
        Some((owner, _)) if owner == who => true, // 就是主人本人
        Some((_, pie)) => !present(pie),          // 主人不在场 ⇒ 可接手
    }
}

/// 这一条挂在**哪一块 `Pane`** 下（`None` = 不在树上）。
///
/// 只读一趟递归（条目规模是几十格，不设缓存——缓存就是第二处真相）：门口那一问要拿"坐标"
/// 去比对主人那份账，而 `trim` 那一支手里只有**号**。
fn container_of(tree: &Operator, id: EntryId) -> Option<Where> {
    fn walk(tree: &Operator, level: &[Where], want: EntryId, at: Where) -> Option<Where> {
        let kids: Vec<EntryId> = tree.list(at).ok()?.collect();
        if kids.contains(&want) {
            return Some(at);
        }
        for kid in kids {
            let next = Where::At(kid);
            if tree.list(next).is_ok()
                && let Some(found) = walk(tree, level, want, next)
            {
                return Some(found);
            }
        }
        None
    }
    walk(tree, &[], id, Where::Root)
}

/// 招待一位客人：从**它的问话孔**读一帧、交给树、把答话推进**它的答话路**。
///
/// 组已经说了"这一枚有话"，故这一读读得动；期限给 `0` 是**再确认**，不是轮询。
fn serve_one(
    tree: &mut Operator,
    guest: Guest,
    session: Option<&Session>,
    publishers: &mut Publishers,
) {
    let Some(ask) = guest.ask() else {
        return;
    };
    let mut buf = [0u8; ocall::ASK_MAX];
    let Ok(n) = mail::HolePie::from_token(ask).pull_timeout(&mut buf, 0) else {
        return;
    };
    let Some(want) = buf.get(..n) else {
        return;
    };
    let mut reply = [0u8; ocall::REPLY_MAX];
    let said = answer(tree, want, guest.who(), session, publishers, &mut reply);
    let _ = mail::HolePie::from_token(guest.reply()).push(&reply[..said]);
}

/// 把一句问交给树，编出一句答（**答话有四种形状**，见 [`ocall`] 的帧那一节）。
///
/// **先读动作码、再按那个动作的形状解载荷**（[`ocall::unpack_ask`]）：解不出来就是一句读不懂的
/// 帧（不猜、不崩）；`land` 那一码**必须带入口号**（没带同样解不出来）。返**帧长**——答案写进
/// 调用方那只缓冲（[`ocall::REPLY_MAX`]）。
fn answer(
    tree: &mut Operator,
    want: &[u8],
    who: TaskId,
    session: Option<&Session>,
    publishers: &mut Publishers,
    out: &mut [u8; ocall::REPLY_MAX],
) -> usize {
    let Some(op) = ocall::op_of(want) else {
        return status(out, ocall::BAD);
    };
    let Some(ask) = ocall::unpack_ask(op, want) else {
        return status(out, ocall::BAD);
    };
    // 路太长：**先按上限挡掉**，别把一条被截断的路当成真的（核心那七条也各有这条判据）。
    if let ocall::AskIn::Road(_, count) = ask {
        if count > Operator::ROAD_MAX {
            return status(out, ocall::FULL);
        }
    }
    // **门外那一问**：两条会**交出权柄 / 毁掉别人那一格**的原语先过门禁——`find`（把那一枚
    // 授出去）与 `trim`（把别人的名字剪掉）。`land` **不在这里**判：它是"改我自己那一格"，
    // 它的准入是**那一格自己的规矩**（见下面的 `publisher`）——先落的人为什么能覆盖后来的人，
    // 反过来却不行？因为"谁能改这一格"是那一格的事实，不是"谁在问"。四条只读结构的
    // （`part` / `list` / `seek` / `name`）一律不判。这一刀的范围见 `docs/operator-gate.md`。
    match ask {
        // **`find` 只看身份，不看主人**——"归落牌那一位"管的是**改这一格**（换绑 / 剪掉），
        // 不是**用这一格**。**实测栽过一次**：把主人判据也挂在 `find` 上，`uart` 声明归自己
        // 之后，`echo` 当场取不到 `/device/uart`——机器还在，控制台没人读（`examine` 0/3）。
        // 这与"公开入口"是同一件事：**读是公开的，写才归属主**。
        ocall::AskIn::Find(id) => {
            let _ = id;
            let ruling = may(session, who);
            if !ruling.passed() {
                return status(out, ruling.wire());
            }
        }
        ocall::AskIn::Trim(id) => {
            if let (Some(at), Some(name)) = (container_of(tree, id), tree.name(id).ok())
                && !claimable(publishers, at, name, who)
            {
                return status(out, ocall::DENIED);
            }
            let ruling = may(session, who);
            if !ruling.passed() {
                return status(out, ruling.wire());
            }
        }
        // **`land` 也要先问身份**（与 `find`/`trim` 同一道门）：它虽然不动别人的格子，
        // 但"往树上挂东西"这件事本身要求来的人是个**已绑身份**——否则没身份的任务就能
        // 往命名空间里塞条目。**实测栽过一次**：漏了这一支，负证客人当场落牌成功
        // （读数 `probe: tree land=OK id=7`）。
        ocall::AskIn::Land { .. } => {
            let ruling = may(session, who);
            if !ruling.passed() {
                return status(out, ruling.wire());
            }
        }
        _ => {}
    }
    let said = match ask {
        // **两条答号的**：立/分的人自己得知道立成了几号——答案体不是一格状态。
        ocall::AskIn::Land {
            at,
            name,
            entry,
            rule,
        } => {
            // **"归落牌的那一位"**：落之前先看这一格现在归谁——不是我就拒（这就是 `land` 那一格
            // 自己的准入）。占了的位置由**先落的人**说了算；空着的位置谁都能落，落了就登记成他的。
            //
            // **按坐标查**（不是按号）：门口这一问发生在动树之前，而 `land` 换绑**不动号**——
            // 见 [`Publishers`] 那段照实记（第一版按号查，真机上把这一道判据整个跳过去了）。
            if !claimable(publishers, at, name, who) {
                return status(out, ocall::DENIED);
            }
            return match tree.land(at, name, entry) {
                Ok(id) => {
                    if rule == ocall::Rule::Owner {
                        // 记的是"**那一刻挂上去的那一枚**"：它答不答得出，就是主人还在不在场。
                        remember(publishers, at, name, who, entry);
                    }
                    ocall::pack_id(out, id)
                }
                Err(fail) => status(out, ocall::fail_to_code(Some(fail))),
            };
        }
        ocall::AskIn::Part { at, name } => {
            return match tree.part(at, name) {
                Ok(id) => ocall::pack_id(out, id),
                Err(fail) => status(out, ocall::fail_to_code(Some(fail))),
            };
        }
        // 查到就**把树上那一份转授给客人**：Pie 不从报文里走，从会话里走。
        // "查不到"与"授不出去"是两件事，故查的结论优先（`.and`）。
        ocall::AskIn::Find(id) => {
            let mut grant = Ok(());
            tree.find(id, |pie| {
                grant = ocall::ship(pie, who).map(|_| ());
            })
            .and(grant)
        }
        ocall::AskIn::Trim(id) => tree.trim(id),
        // **三条答数据的**：答案体不是一格状态，故各自编各自的帧（成败都在帧里）。
        ocall::AskIn::List(at) => {
            return match tree.list(at) {
                Ok(ids) => ocall::pack_list(out, ids),
                Err(fail) => status(out, ocall::fail_to_code(Some(fail))),
            };
        }
        ocall::AskIn::Name(id) => {
            return match tree.name(id) {
                Ok(name) => ocall::pack_name(out, name),
                Err(fail) => status(out, ocall::fail_to_code(Some(fail))),
            };
        }
        // **译号那一档**：名字只能走到这里——拿到号之后，其余原语一律按号走。
        ocall::AskIn::Road(road, count) => {
            return match tree.seek(&road[..count.min(Operator::ROAD_MAX)]) {
                Ok(id) => ocall::pack_id(out, id),
                Err(fail) => status(out, ocall::fail_to_code(Some(fail))),
            };
        }
    };
    status(out, ocall::fail_to_code(said.err()))
}

/// 一格状态的答：写进 `out` 的第一格，返 1。
fn status(out: &mut [u8; ocall::REPLY_MAX], code: u8) -> usize {
    out[0] = code;
    1
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
    let link = Mark::of(LINK);
    let mut index = 0usize;
    loop {
        let (token, _perm, vestor) = mail::collect(index).ok()?;
        // 越界哨兵：这一遍扫完了。
        if token.get() == 0 {
            return None;
        }
        index += 1;
        let _ = vestor;
        if ocall::opened_by(token) == Some(who) && ocall::marked_as(token) == Some(link) {
            return Some(token);
        }
    }
}

/// 这一位客人**自己**交来的那一枚问话孔。
///
/// 判据两格，缺一不可：`owner == who`（那扇门是它开的）**且** 记号 == `ask`（它亲手铸的
/// 那一枚）——客人交来的**入口**也满足前两格（都是它铸、它交的），两件事只有记号分得开。
fn ask_of(who: TaskId) -> Option<PieToken> {
    let ask = ASK_MARK;
    let mut index = 0usize;
    loop {
        let (token, _perm, _vestor) = mail::collect(index).ok()?;
        // 越界哨兵：这一遍扫完了。
        if token.get() == 0 {
            return None;
        }
        index += 1;
        if ocall::opened_by(token) == Some(who) && ocall::marked_as(token) == Some(ask) {
            return Some(token);
        }
    }
}

/// 持树者的读数：**只在出岔子时说话**（正常一轮什么都不打）。
fn say(msg: &str) {
    let _ = runtime::env::debug::put(msg);
}
