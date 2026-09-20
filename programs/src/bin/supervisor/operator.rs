//! operator — **那棵树的两半**：持树者侧（[`serve`] 服务循环 + [`attach`] 装配）与客侧
//! （[`open`] / [`ask_hole`] / [`ask`] / [`take`]）。
//!
//! 与板那一台（`supervisor/board.rs`）同一个形状：**一枚线程招待所有客人**，两侧都用会话的
//! 同一对动作（`seat` + `claim`），靠**孔上的记号**对位。
//!
//! ```text
//!   装配者（root）                            客人（某个服务域）        持树者（operator 域，一枚线程）
//!   quay.seat(LINK) + quay.claim(客人, LINK) ▶ open: seat(LINK) + claim(生我者, LINK)
//!   转授：把客人那一枚 Ship 给持树者 ──────────────────────────────────▶  按"谁转授的 + 记号"认答话写端
//!   LINK 上先递一格：持树者的号 ────────────▶  open 收下 ⇒ 此后叫得出它
//!   提示：往提示之路推一个客人号 ─────────────────────────────────────▶  收一位客人（admit）
//!                                            铸问话孔(ask)、Ship 给持树者 ▶  按"谁开的 + 记号"认出 ⇒ arm + hang
//!                                            推(Ask) ───────────────────▶  组唤醒 ⇒ pull ⇒ 交给树
//!                                            读(Reply) ◀────────────────  推(Reply)
//! ```
//!
//! # 为什么持树者就一枚线程
//!
//! 树上那几枚是**一枚只在持它的那张表里有意义的句柄**（`PieToken` = "我这张表里的第几个"）。
//! "查到了要把 Pie 授出去"必须由**持有那一枚的那张表**来做——故所有条目只能住同一张表，
//! 也就是同一枚线程。板那一台栽过这条（每位客人一枚待客线程 ⇒ 甲的条目在甲的表里，乙来查
//! 时判它"已死"、也授不出去，症状是"刚挂上的名字，别人一查就是 `Unknown`"）。
//!
//! # 与板那一台的差别（照实记）
//!
//! 1. **持树者住在自己的域里**（板线程住 root 域）：故它是被 `service::spawn` 产出来的，
//!    装配者从 `rep` 就知道它是谁，不必再起线程；它那一枚提示孔由**它自己**铸、Ship 给
//!    生我者（= 装配者 = 那一枚 `Quay::claim` 的对端）。
//! 2. **没有"客人说走了"那一档**：本正文里没有客人生命周期（谁问都答）。故没有退场的
//!    那一格——只有"看出来"那一档（那一枚答不出 ⇒ 剔格子）。
//! 3. **没有死亡道**：板那是给 root 监督服务用的（一位服务一条 `gone-<名字>`）。树这一侧
//!    不推任何东西给装配者。
//! 4. **帧里带一整条路**（段列表），不是单个名字；入口那一枚仍然经会话交出去，报文里只有
//!    一格状态。
//!
//! # 入口那一枚不经装配者转授
//!
//! 客人要跟持树者说话，只需两样：**它是谁**（号）与**一条答话路**。号由装配者递一格告诉它
//! （[`attach`] 的 `tell`），答话路由装配者转授给持树者。**客人递 Pie 给持树者不必先拿到
//! 什么凭证**——`Accord` 的目的地就是一个 `TaskId`，故"把 Pie 交出去"这一步是客人直接对
//! 持树者做的（板那一台也是这么交入口的）。

use env::{HoleDir, Name, PieToken, TaskId};
use protocol::operator::call as ocall;
use protocol::operator::{Desk, Fail, Guest, Operator};
use protocol::session::Quay;
use runtime::core::port::{self, Access, Policy};
use runtime::core::tole::Tole;
use runtime::env::mail::{self, AnyPie};
use runtime::env::room::exit_with;
use runtime::env::unit as utask;

/// 树那条通道的名字：**两侧同一个**（泊位自己的坐标，不进报文）。
pub const LINK: &str = "operator";

/// 问话孔那一枚上的记号（两侧同一个：客人铸它时刻上去的，持树者按它认领那枚孔）。
const ASK_MARK: &str = "ask";

/// 提示孔那一枚上的记号（持树者铸它时刻上去的；装配者按它认领那一枚）。
const TIP_MARK: &str = "tip";

/// 提示之路的名字（只有装配者那侧用得上：持树者那一枚是它自己铸的，不需要名字）。
const TIP_NAME: &str = "operator-tip";

/// 还在"补齐两本账"（答话路未认领 / 问话孔未挂上）时，一轮等多久（毫秒）。
///
/// **不是轮询**：账补齐之后这一等就变成 `usize::MAX`（由组唤醒）；这个短期限只在装配窗口
/// 里用——那几步的到达是**别人**在做（装配者转授、客人自己交孔）。
const SETTLE_MS: usize = 1;

/// 持树者起不来时的编号（指"死在头几步的哪一步"）。
const E_SIRE: usize = 1;
const E_TIP: usize = 2;
const E_GROUP: usize = 3;

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
    if tole.hang(&tip_hole, HoleDir::Pull).is_err() {
        say("operator: tip not hung");
        exit_with(E_GROUP);
    }

    let mut tree = ocall::tree();
    let mut desk = ocall::desk();
    loop {
        // 一、补齐两件事（收提示 + 认领答话路、认出问话孔并挂组）。
        let settling = settle(&mut desk, assembler, &tole, &tip_hole);
        // 二、等一格有事。**一个等待**：提示孔或任意一位客人的问话孔。
        let ms = if settling { SETTLE_MS } else { usize::MAX };
        let Ok(Some((tok, _dir))) = tole.await_(ms) else {
            let _ = desk.sweep();
            continue;
        };
        // 提示孔那一格由下一轮的 `settle` 收（它非阻塞地拉）；这里只管"是哪位客人的问话孔"。
        if tok != tip
            && let Some(guest) = desk.guest(tok).copied()
        {
            serve_one(&mut tree, guest);
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
/// - **答话路**：装配者转授来的那一枚 ⇒ `admit` 收一位客人；
/// - **问话孔**：客人**自己**交来的那一枚 ⇒ 认出来就 `arm` + 挂进组。
fn settle(desk: &mut Desk, assembler: TaskId, tole: &Tole, tip: &mail::HolePie) -> bool {
    // 提示：拉干净（单槽，一位客人一条）。**非阻塞**——它的到达是别人在做的事。
    let mut id = [0u8; 8];
    let mut pending = false;
    while let Ok(8) = tip.pull_timeout(&mut id, 0) {
        let client = TaskId::new(u64::from_le_bytes(id) as usize);
        match reply_of(assembler, client) {
            // 已经在账上（提示是单槽，可能重放）：不换掉原来那位。
            Some(reply) => {
                let _ = desk.admit(client, reply);
            }
            // 次序被破坏（提示先到、答话路不在本表里）：报一句；客人那边会报它自己的超时。
            None => say("operator: no reply"),
        }
    }
    // 先抄一份"还没挂上的"：`unarmed` 借住这本账，而下面要改它。
    let mut waiting = [(0usize, TaskId::new(0)); Desk::CAP];
    let mut n = 0;
    for (slot, who) in desk.unarmed() {
        waiting[n] = (slot, who);
        n += 1;
    }
    for &(slot, who) in &waiting[..n] {
        match ask_of(who) {
            Some(ask) => {
                let hung = desk.arm(slot, ask).is_ok()
                    && tole
                        .hang(&mail::HolePie::from_token(ask), HoleDir::Pull)
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

/// 招待一位客人：从**它的问话孔**读一帧、交给树、把答话推进**它的答话路**。
///
/// 组已经说了"这一枚有话"，故这一读读得动；期限给 `0` 是**再确认**，不是轮询。
fn serve_one(tree: &mut Operator, guest: Guest) {
    let Some(ask) = guest.ask() else {
        return;
    };
    let mut buf = [0u8; ocall::ASK];
    let Ok(n) = mail::HolePie::from_token(ask).pull_timeout(&mut buf, 0) else {
        return;
    };
    let Some(want) = buf.get(..n) else {
        return;
    };
    let answer = answer(tree, want, guest.who());
    let _ = mail::HolePie::from_token(guest.reply()).push(&answer);
}

/// 把一句问交给树，编出一句答（**一格**：读不懂也答，答 `BAD`）。
///
/// **先读动作码、再解载荷**；`file` 那一码**必须带入口号**（没带就是一句读不懂的 `file`：
/// 不猜、不崩）。
fn answer(tree: &mut Operator, want: &[u8], who: TaskId) -> [u8; 1] {
    let Some(op) = ocall::op_of(want) else {
        return [ocall::BAD];
    };
    let Some((segs, count, seed)) = ocall::unpack(want) else {
        return [ocall::BAD];
    };
    // 路太长：**先按上限挡掉**，别把一条被截断的路当成真的（核心那四条也各有这条判据）。
    if count > Operator::PATH_MAX {
        return [ocall::FULL];
    }
    let path = &segs[..count];
    // 一句没带入口号的 `file`：这不是"树上没有这一段"，是**这一问读不懂** ⇒ `BAD`。
    if op == ocall::FILE && seed == PieToken::NONE {
        return [ocall::BAD];
    }
    let said = match op {
        ocall::FILE => tree.file(path, seed),
        ocall::TILE => tree.tile(path),
        // 查到就**把树上那一份转授给客人**：Pie 不从报文里走，从会话里走。
        // "查不到"与"授不出去"是两件事，故查的结论优先（`.and`）。
        ocall::FIND => {
            let mut grant = Ok(());
            tree.find(path, |pie| {
                grant = ocall::give(pie, who).map(|_| ());
            })
            .and(grant)
        }
        ocall::TRIM => tree.trim(path),
        // 没见过的动作码：与"这条路上没有这一段"同一句话（不另立一格）。
        _ => Err(Fail::Unknown),
    };
    [ocall::code(said.err())]
}

// ── 装配侧（装配者调用）──────────────────────────────────────

/// 把持树者接上一位客人（装配者调用）：**三步**（见文件头那张图）。
///
/// `host` = 持树者的号（`service::spawn` 交回来的那个，装配者本来就知道它）。
/// `tip` = 提示之路在**本线程表里**的那一枚（第一次用时认下来，此后逐条传下去）。
///
/// 返 `Err(哪一步)`：名字非法 / 席位满 / 等不到客人那一枚 / 提示孔认不到……对调用方是
/// 同一件事——**这条服务没接上树**——但"死在哪一步"正是装配诊断要的那一格。
pub fn attach(
    quay: &mut Quay,
    client: TaskId,
    host: TaskId,
    ms: usize,
    tip: &mut Option<PieToken>,
) -> Result<(), &'static str> {
    let link = Name::new(LINK).map_err(|_| "operator:name")?;
    // 1. 本端那一枚交出去（落在本域表里——客人拿不到它，也不需要：答话从客人自己那枚走）。
    quay.seat(link).map_err(|_| "operator:seat")?;
    // 2. 认领**这位客人**交出来的那一枚（记号 = 这条路的名字，客侧 `seat` 刻的就是它）。
    quay.claim(client, link, ms).map_err(|_| "operator:claim")?;
    // 3. 提示孔（只认一次）→ 把客人那一枚转授给持树者 → 告两边。
    let _ = host_of(host, ms, tip)?;
    let reply = reply_path(quay).ok_or("operator:hand")?;
    hand(reply, host).map_err(|()| "operator:hand")?;
    // 客人那一侧的一格：**答话的是谁**（持树者的号，8 字节）。
    tell(host, reply).map_err(|_| "operator:who")?;
    // 提示在**转授之后**：持树者据此可以按"提示一到，答话路必已在本表里"办事。
    tell(client, (*tip).ok_or("operator:tip")?).map_err(|_| "operator:tell")
}

/// 认下持树者交回来的那一枚提示孔（**只认一次**）：本域另开一座码头等它。
///
/// 判据两格：`owner == 持树者`（那一枚是它铸的）**且** 记号 = `tip`。认下来之后本线程
/// 拿着的就是"往提示之路推客人号"那一枚。
fn host_of(host: TaskId, ms: usize, tip: &mut Option<PieToken>) -> Result<TaskId, &'static str> {
    if tip.is_some() {
        return Ok(host);
    }
    let slot = Name::new(TIP_NAME).map_err(|_| "operator:name")?;
    let mark = Name::new(TIP_MARK).map_err(|_| "operator:name")?;
    let mut quay = Quay::open(host);
    quay.seat(slot).map_err(|_| "operator:seat")?;
    quay.claim(host, mark, ms).map_err(|_| "operator:tip")?;
    let pier = quay.find(slot).ok_or("operator:tip")?;
    // 交给调用方拿着：同一条路上以后每次都往里推客人号（**同一枚线程**用它）。
    *tip = pier.at_peer();
    if tip.is_none() {
        return Err("operator:tip");
    }
    Ok(host)
}

/// 把一个号推过去（8 字节，小端）。
///
/// **两处共用这一句**：提示孔那一格（告持树者"客人是谁"）与树路那一格（告客人"答话的是谁"）。
/// 两处都是"装配者知道、对方叫不出"的那个号——故 `tell` 只认"推给哪一枚孔"，不认语义。
fn tell(who: TaskId, into: PieToken) -> Result<(), ()> {
    let into = mail::HolePie::from_token(into);
    into.push(&(who.get() as u64).to_le_bytes()).map_err(|_| ())
}

/// 收下树路上那一格：**答话的是谁**（[`tell`] 的对偶）。
///
/// 返 `None` = 期限到了还没到 ⇒ 这条服务没接上树（客人报它自己的超时，不猜）。
fn hear(quay: &Quay, ms: usize) -> Option<TaskId> {
    let link = Name::new(LINK).ok()?;
    let pier = quay.find(link)?;
    let mut buf = [0u8; 8];
    match mail::HolePie::from_token(pier.hole()).pull_timeout(&mut buf, ms) {
        Ok(8) => Some(TaskId::new(u64::from_le_bytes(buf) as usize)),
        _ => None,
    }
}

/// 树路上本端手里那一枚（客人答话路的**写端**）：答话往它推，"答话的是谁"也从它递。
fn reply_path(quay: &Quay) -> Option<PieToken> {
    let link = Name::new(LINK).ok()?;
    quay.find(link)?.at_peer()
}

/// 把**客人交出来的那一枚**转授给持树者。
///
/// 转授的是"客人开的那扇门"（`owner` 是客人），持树者那侧认领时认的正是它。
///
/// 子集只给 `R|W`，**不加 `VEST`**：持树者用这一枚写答话，不需要再授出——一分不多。
fn hand(reply: PieToken, host: TaskId) -> Result<(), ()> {
    let hole = mail::HolePie::from_token(reply);
    port::ship(&hole, host, Access::FETCH | Access::STORE, Policy::NONE)
        .map(|_| ())
        .map_err(|_| ())
}

/// 装配者转授来的那一枚答话路（**写端**，落在本表里）。
///
/// 三格判据，都是确定的号：
///
/// - `vestor == assembler` —— **谁转的**（这一格把"客人自己交来的孔"分开）；
/// - `owner == who` —— **谁的**：那扇门是**这位客人**开的（副本共享同一事实）；
/// - **记号 == `operator`** —— 那一枚是**树路**上的一枚。
fn reply_of(assembler: TaskId, who: TaskId) -> Option<PieToken> {
    let link = Name::new(LINK).ok()?;
    let mut index = 0usize;
    loop {
        let (token, _perm, vestor) = mail::collect(index).ok()?;
        // 越界哨兵：这一遍扫完了。
        if token.get() == 0 {
            return None;
        }
        index += 1;
        if vestor == assembler
            && ocall::opened_by(token) == Some(who)
            && ocall::mark_of(token) == Some(link)
        {
            return Some(token);
        }
    }
}

/// 这一位客人**自己**交来的那一枚问话孔。
///
/// 判据两格，缺一不可：`owner == who`（那扇门是它开的）**且** 记号 == `ask`（它亲手铸的
/// 那一枚）——客人交来的**入口**也满足前两格（都是它铸、它交的），两件事只有记号分得开。
fn ask_of(who: TaskId) -> Option<PieToken> {
    let ask = Name::new(ASK_MARK).ok()?;
    let mut index = 0usize;
    loop {
        let (token, _perm, _vestor) = mail::collect(index).ok()?;
        // 越界哨兵：这一遍扫完了。
        if token.get() == 0 {
            return None;
        }
        index += 1;
        if ocall::opened_by(token) == Some(who) && ocall::mark_of(token) == Some(ask) {
            return Some(token);
        }
    }
}

/// 持树者的读数：**只在出岔子时说话**（正常一轮什么都不打）。
fn say(msg: &str) {
    let _ = runtime::env::debug::put(msg);
}

// ── 客侧（服务域）────────────────────────────────────────────

/// 客侧第一步：装上树那条路（**记号就是这条路的名字**），认下对端那一枚，并收下
/// "**答话的是谁**"（[`hear`] 那一格）。
///
/// 返本端这座码头（**答话**从它走，问话走 [`ask_hole`]）与**持树者的号**。
///
/// `holder` = 客人认的对端 = **它的生我者**（孔交给它，它再转授给持树者）——注意它不是
/// 持树者：客人交出来的孔都落在生我者表里，故"持树者是谁"得由装配者告诉（见文件头）。
pub fn open(holder: TaskId, ms: usize) -> Result<(Quay, TaskId), Fail> {
    let link = Name::new(LINK).map_err(|_| Fail::Unknown)?;
    let mut quay = Quay::open(holder);
    quay.seat(link).map_err(ocall::map_seat)?;
    quay.claim(holder, link, ms).map_err(ocall::map_claim)?;
    let host = hear(&quay, ms).ok_or(Fail::Unknown)?;
    Ok((quay, host))
}

/// 客侧第一步半：铸**问话孔**并交到持树者手里（本端随即自窄到只写）。
///
/// `host` = [`open`] 收下的那个号。交出去的是可读可写，随后本端 `narrow` 到 `STORE`：
/// 一条路上只有一个读者，故**持树者读、本端写**。
///
/// 记号 = [`ASK_MARK`]：持树者那侧就是按它把这枚孔与**入口**分开的（两枚都由本端铸、本端交）。
pub fn ask_hole(host: TaskId) -> Result<PieToken, Fail> {
    let ask = mail::unseal_hole(ASK_MARK).map_err(|_| Fail::Unknown)?;
    let hole = mail::HolePie::from_token(ask);
    port::ship(&hole, host, Access::FETCH | Access::STORE, Policy::NONE)
        .map_err(|_| Fail::Unknown)?;
    hole.narrow(env::Permission::STORE)
        .map_err(|_| Fail::Unknown)?;
    Ok(ask)
}

/// 客侧第二步：问一句、取一句答。返答话那一格（[`ocall::OK`] = 持树者收下了）。
///
/// 问话推 `say`（[`ask_hole`] 铸的那一枚，持树者读），答话从本端这条树路读（持树者写）。
///
/// `file` 那一码要把**入口**捎上：它经会话交给持树者（`Accord` 一份），故写进帧里的是
/// "种在持树者表里的那个号"——那个号才是它认得的坐标。
pub fn ask(
    say: PieToken,
    link: &Quay,
    host: TaskId,
    op: u8,
    path: &[Name],
    entry: PieToken,
    ms: usize,
) -> Result<u8, Fail> {
    let at = Name::new(LINK).map_err(|_| Fail::Unknown)?;
    let pier = link.find(at).ok_or(Fail::Unknown)?;
    let seed = match op {
        ocall::FILE => Some(ocall::hang_in(entry, host).map_err(|()| Fail::Unknown)?),
        _ => None,
    };
    // 孔是单槽：槽里还压着上一条时这一推会**等在门外**（`push` 满则挂），不是错误。
    mail::HolePie::from_token(say)
        .push(&ocall::pack(op, path, seed))
        .map_err(|_| Fail::Unknown)?;
    let mut reply = [0u8; 1];
    match pier.pull(&mut reply, ms) {
        Ok(1) => Ok(reply[0]),
        _ => Err(Fail::Unknown),
    }
}

/// 客侧第三步：把**刚授进来的那一枚**从本端表里取出来（`find` 的下场）。
///
/// 一格判据：**来源位是持树者**（这一份是它交给本端的——`Reply` 里没有号，故只能按"谁给的"
/// 认），取满足的那些里**最后**一枚（表按登记先后枚举；一次一问一答只授一枚）。
///
/// **`owner` 在这里没用**：那扇门是**别人**开的（树上那一条是谁挂的，开者就是谁），客人不是
/// 它的开者。
pub fn take(link: &Quay, host: TaskId) -> Option<PieToken> {
    let at = Name::new(LINK).ok()?;
    let _ = link.find(at)?;
    let mut index = 0usize;
    let mut found = None;
    loop {
        let (token, _perm, vestor) = mail::collect(index).ok()?;
        // 越界哨兵：这一遍扫完了。
        if token.get() == 0 {
            return found;
        }
        index += 1;
        if vestor == host {
            found = Some(token);
        }
    }
}
