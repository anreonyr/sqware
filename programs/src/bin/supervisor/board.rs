//! board — **板那两半**：板侧（[`attach`] 起板线程 + 转授），客侧（[`open`] / [`ask`] / [`take`]）。
//!
//! 板是**装配者那个域里的一枚线程**（就一枚，招待所有客人），客人是别的域里的线程。
//! 两侧各用会话的一侧（[`Quay::pair`] 的两半）：
//!
//! ```text
//!   装配者（root）                            客人（服务域）            板线程（一枚）
//!   quay.seat(板路) + quay.claim(客人) ────▶ pair(板路) ── 交一枚孔 ──▶  按"谁转授的"认领答话写端
//!   转授：把客人那一枚 Ship 给板线程 ─────────────────────────────────▶  答话路的写端到手
//!   板路上先递一格：板线程的号 ────────────▶  open 收下 ⇒ 此后叫得出板
//!   提示：往提示之路推一个客人号 ────────────────────────────────────▶  收一位客人（admit）
//!                                            铸问话孔、Ship 给板 ──────▶  按"谁交的"认出 ⇒ arm + hang
//!                                            推(Query) ────────────────▶  组唤醒 ⇒ pull ⇒ 交给板
//!                                            读(Reply) ◀───────────────  推(Reply)
//! ```
//!
//! **两条路各一枚孔，两枚都落进板的表里**：答话那条的**写端**由装配者转授（`pair` 建立的
//! 会话），问话那条由**客人自己铸**再交给板。板按"**这枚是谁交给我的**"认领（来源位
//! `vestor`），与 `register` 同一判据——客人是**唯一"每位客人各不同"的身份**，故板只能按
//! 它分人；板自己铸则八位客人的孔同形，反倒认不出是哪位的。
//!
//! **两枚孔都要客人先叫得出板**：孔是"铸的人那张表里的第几个"，换一张表就念不出来。客人
//! 只认得生我者（装配者），故装配者在**板路上先递一格 8 字节：答话的是谁**——与提示孔那
//! 一格对偶（告板"客人是谁"，也告客人"答话的是谁"）。缺这一格，客人交出来的孔就落进装配
//! 者表里，而装配者不代转：板永远收不到问话（实测症状：板每轮都是 `settling`，客人超时）。
//!
//! 板把**所有**问话孔挂进**一个组**，于是"等 N 位客人说话"是一件事，不是一个轮询圈。
//!
//! **为什么中间要装配者过一手**：客人只认得它的生我者（孔是交给"生我者"的），板线程不是
//! 它的生我者 ⇒ 客人交出来的那一枚落在装配者表里。装配者把它**转授**给板线程——装配者本来
//! 就是设备门闩的第一个持有者与转授者，这里走的是同一条路。转授的是**客人开的那扇门**：
//! 副本共享 `owner`（`session` 事实 3），故板那一侧照样念得出"这是哪位的孔"。
//!
//! # 板为什么就一枚线程
//!
//! 牌子上的入口是**一枚只在对端表里有意义的句柄**（`PieToken` = "我这张表里的第几个"）。
//! "查到了要把入口授出去"必须由**持有那一枚的那张表**来做——故所有牌子只能住同一张表，
//! 也就是同一枚线程。先前"一位客人一枚待客线程"的形状正是栽在这里：甲的入口在甲的板线程
//! 表里，乙来查时乙的板线程手里没有它 ⇒ `probe` 判它"已死"（牌子当场被扫空）、`give` 也
//! 交不出去。**症状是"刚挂上的名字，别人一查就是 `Unknown`"**。
//!
//! 一枚线程招待多位客人靠**一个组**（[`Tole`]）：提示孔 + 每位客人的问话孔，一格一枚，
//! 全挂同一个组 ⇒ 板线程只有**一个等待**，醒来手里就是"哪一枚有话"（[`Desk::guest`]
//! 直达）。没有轮询、没有一圈 N 毫秒、也没有"N 路探测"。
//!
//! # 一问一答的次序
//!
//! ```text
//!   1  装配者在本域与客人那座码头上加一条板泊位（本端交出自己那一枚 —— 答话从它走）
//!   2  认领：**这位客人**交出来的那一枚落进本域表里（客人只认得生我者，故先落这里）
//!   3  板线程（只起一枚）→ 把客人那一枚转授给它 → 板路上递一格"答话的是谁" → 提示来客人了
//! ```
//!
//! 三步都在 [`attach`] 里，**次序即契约**：第 3 步之前板线程还没起（起了也无用——它等的
//! 就是这一枚），而客人那一枚要到第 2 步才在。第 3 步里**转授在前、提示在后**，于是板可以
//! 按一句硬话办事：**提示一到，答话路必已在本表里**（它不必为一枚还没到的孔留格子）。
//!
//! # 字节长什么样
//!
//! 帧形、三个动作码、一处上界、答话那一格全在 [`protocol::board::call`]；本文件只做
//! "读一条 → 交给板 → 回一句"，一个字节都不自己编。

extern crate alloc;

use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::format;

use env::{HoleDir, Name, PieToken, TaskId};
use protocol::board::call as bcall;
use protocol::board::{Board, Desk, Fail};
use protocol::session::Quay;
use runtime::core::port::{self, Access, Policy};
use runtime::core::tole::Tole;
use runtime::core::unit::{self, Join};
use runtime::env::mail::{self, AnyPie};

/// 板那条通道的名字：**两侧同一个**（泊位自己的坐标，不进报文）。
pub const LINK: &str = "board";

/// 提示之路的名字（只有装配者那侧用得上：板线程那一枚是它自己铸的，不需要名字）。
const TIP_NAME: &str = "board-tip";

/// 还在"补齐两本账"（答话路未认领 / 问话孔未挂上）时，一轮等多久（毫秒）。
///
/// **不是轮询**：账补齐之后这一等就变成 `usize::MAX`（无限等，由组唤醒）；这个短期限只在
/// 装配窗口里用——那几步的到达是**别人**在做（装配者转授、客人自己交孔），板线程没有别的
/// 东西可等。
const SETTLE_MS: usize = 1;

/// 板线程的号（0 = 还没起）。`TaskId` 是**全局身份**，可跨线程，故这一枚放得进 static。
static HOST: AtomicUsize = AtomicUsize::new(0);
// 提示之路在**装配者表里**的那一枚句柄**不放 static**：`PieToken` 是表的身份、标着 `!Sync`
// （`env::wire::handle`），static 装不下它——它由装配者这一枚线程自己拿着，逐次传下去。

// ── 板侧（装配者与本域的板线程）──────────────────────────────

/// 把板接上一位客人（装配者调用）：**三步**（见文件头"一问一答的次序"）。
///
/// `client` = 客人（服务域的主线程 = 装配者刚产出的代表线程）：**客人交出来的那一枚就落在
/// 本域表里**（客人把孔交给生我者），而它的 `owner` 正是这位客人——故第 2 步认的是**它**。
///
/// 返 `Err(哪一步)`：三种因（名字非法 / 席位满 / 等不到客人的那一枚）对调用方是同一件事
/// ——**这条服务没接上板**——但"死在哪一步"正是装配诊断要的那一格（与 `service::step` 同款）。
pub fn attach(
    quay: &mut Quay,
    me: TaskId,
    client: TaskId,
    ms: usize,
    tip: &mut Option<PieToken>,
) -> Result<(), &'static str> {
    let link = Name::new(LINK).map_err(|_| "board:name")?;
    // 1. 本端那一枚交出去（落在本域表里——客人拿不到它，也不需要：答话从客人自己那枚走）。
    quay.seat(link, ms).map_err(|_| "board:seat")?;
    // 2. 认领**这位客人**交出来的那一枚。本域给每个孩子各开一座码头，故认的是"它给我的"，
    //    不是"我没开过的"——不然会把别的客人的孔配到它头上（`Quay::claim` 的正文）。
    //    **次序**：板那条比 `records` 后到，而 `records` 的写端已经用掉了 ⇒ 这一枚认在
    //    板泊位上（一枚孔只配一条泊位）。
    quay.claim(client, ms).map_err(|_| "board:claim")?;
    // 3. 板线程（只起一枚）→ 把客人那一枚转授过去 → 板路上递一格"答话的是谁" → 提示来客人了。
    let host = host(me, ms, tip)?;
    let Some(tip) = *tip else {
        return Err("board:tip");
    };
    let reply = reply_path(quay).ok_or("board:hand")?;
    hand(reply, host).map_err(|()| "board:hand")?;
    // 客人那一侧的一格：**答话的是谁**（板线程的号，8 字节）——与提示孔那一格对偶。
    tell(host, reply).map_err(|_| "board:who")?;
    // 提示在**转授之后**：板据此可以按"提示一到，答话路必已在本表里"办事。
    tell(client, tip).map_err(|_| "board:tell")
}

/// 起板线程（**就一枚**），返它的号；起过了就把那个号给回来。
///
/// "为什么就一枚"见文件头。这里补**提示之路**的来历：装配者得告诉板线程"来客人了、它是
/// 谁"，而孔是**铸的人那张表**里的东西（装配者铸的孔，板线程表里没有它）——故这条路由板
/// 线程自己铸：它起来第一件事就是把这枚孔的副本交给**装配者**。`me` 因此得从外面给：
/// 同域里产出来的线程，`sire` 是**域的**生我者（建这个域的那一枚），不是产它的那一枚
/// （`UnitCall::Sire` 的正文）——同一个域里的两枚线程，"谁生我"答不出"谁产的"。
fn host(me: TaskId, ms: usize, tip: &mut Option<PieToken>) -> Result<TaskId, &'static str> {
    let had = HOST.load(Ordering::Acquire);
    if had != 0 {
        return Ok(TaskId::new(had));
    }
    let node: Join<()> = unit::closure(move || host_loop(me));
    let id = node.id();
    // **弃权**（`Join` 的 Drop）：不等它的结果，但按协议把完成盒子交回去（板线程长期不
    // 返回；`mem::forget` 会把它漏掉）。
    drop(node);
    HOST.store(id.get(), Ordering::Release);

    // 认领板线程交回来的那一枚提示孔：本域另开一座码头等它（判据 `owner == 板线程`）。
    // 这条路上只走"客人号"，故本端那一枚交出去也无妨（板线程不用它，也不碍事）。
    let slot = Name::new(TIP_NAME).map_err(|_| "board:name")?;
    let mut quay = Quay::open(id);
    quay.seat(slot, ms).map_err(|_| "board:seat")?;
    quay.claim(id, ms).map_err(|_| "board:tip")?;
    let pier = quay.find(slot).ok_or("board:tip")?;
    // 交给调用方拿着：同一条路上以后每次都往里推客人号（**同一枚线程**用它）。
    *tip = Some(pier.at_peer());
    Ok(id)
}

/// 把一个号推过去（8 字节，小端）。
///
/// **两处共用这一句**：提示孔那一格（告板"客人是谁"）与板路那一格（告客人"答话的是谁"）。
/// 两处都是"装配者知道、对方叫不出"的那个号——故 `tell` 只认"推给哪一枚孔"，不认语义。
fn tell(who: TaskId, into: PieToken) -> Result<(), ()> {
    let into = mail::HolePie::from_token(into);
    into.push(&(who.get() as u64).to_le_bytes()).map_err(|_| ())
}

/// 收下板路上那一格：**答话的是谁**（[`tell`] 的对偶）。
///
/// 返 `None` = 期限到了还没到 ⇒ 这条服务没接上板（客人报它自己的超时，不猜）。
fn hear(quay: &Quay, ms: usize) -> Option<TaskId> {
    let link = Name::new(LINK).ok()?;
    let pier = quay.find_pier(link)?;
    let mut buf = [0u8; 8];
    match mail::HolePie::from_token(pier.hole()).pull_timeout(&mut buf, ms) {
        Ok(8) => Some(TaskId::new(u64::from_le_bytes(buf) as usize)),
        _ => None,
    }
}

/// 板路上本端手里那一枚（客人答话路的**写端**）：答话往它推，"答话的是谁"也从它递。
fn reply_path(quay: &Quay) -> Option<PieToken> {
    let link = Name::new(LINK).ok()?;
    Some(quay.find(link)?.at_peer())
}

/// 把**客人交出来的那一枚**转授给板线程。
///
/// 转授的是"客人开的那扇门"（`owner` 是客人），板那侧认领时认的正是它。
///
/// 子集只给 `R|W`，**不加 `VEST`**：板线程用这一枚写答话，不需要再授出——一分不多。
/// 本域自己那一份转授之后**不收**：内核那侧"交出"（`CAGE`）要求源枚自己带 `CAGE`，而客人
/// 给过来的这一枚没有（`seat` 给的是 `R|W|VEST`）；收它要多一条 `release`，而这一步之后
/// 没有任何东西再碰它——本域常驻，随域退场一起回收。
fn hand(reply: PieToken, host: TaskId) -> Result<(), ()> {
    let hole = mail::HolePie::from_token(reply);
    port::ship(&hole, host, Access::READ | Access::WRITE, Policy::NONE)
        .map(|_| ())
        .map_err(|_| ())
}

/// 板线程：**一枚线程招待所有客人**。
///
/// ```text
///   起手  铸一枚提示孔（副本交给装配者），把它与所有问话孔挂进**同一个组**
///   循环  补齐两件事（收提示 + 认领答话路 / 认出问话孔并挂组）
///         等一格有事（一个等待）—— 提示孔 ⇒ 来客人了；问话孔 ⇒ 读一帧、答一句
///         惰性剔走已经走了的客人
/// ```
///
/// **板不用码头**：它要的两枚孔都不是它开的——答话路的**写端**是装配者转授进来的，问话孔的
/// **读端**是客人交进来的，两枚都按来源位（`vestor`）在本表里认出来。铸一枚孔就会给对端
/// 送去一枚用不上的写端（那是一枚"码头"，客人表里的渣），故这里一枚也不铸（提示孔除外：
/// 它的副本正是装配者要的那枚）。
fn host_loop(me: TaskId) {
    // `me` = **装配者**（不是本线程的号）：答话路是它转授来的，故判据的一半是它。
    // 提示孔：本线程铸的那一枚（客人号从这里进来），副本交给装配者。
    let Ok(tip) = mail::unseal_hole() else {
        say("board: no tip");
        return;
    };
    let tip_hole = mail::HolePie::from_token(tip);
    if port::ship(&tip_hole, me, Access::READ | Access::WRITE, Policy::VEST).is_err() {
        say("board: tip not handed");
        return;
    }
    // **一个组**：提示孔 + 每位客人的问话孔。提示孔也挂进来，故"来客人了"与"有人问话"
    // 是**同一个等待**——这正是"等 N 位客人说话"要的那一格。
    let Ok(tole) = Tole::unseal() else {
        say("board: no group");
        return;
    };
    if tole.hang(&tip_hole, HoleDir::Pull).is_err() {
        say("board: tip not hung");
        return;
    }

    let mut board = bcall::board();
    let mut desk = bcall::desk();
    loop {
        // 一、补齐两件事（收提示 + 认领答话路、认出问话孔并挂组）。还有没补齐的就只等一小段。
        let settling = settle(&mut desk, me, &tole, &tip_hole);
        // 二、等一格有事。**一个等待**：提示孔或任意一位客人的问话孔。
        let ms = if settling { SETTLE_MS } else { usize::MAX };
        let Ok(Some((tok, _dir))) = tole.await_(ms) else {
            desk.sweep();
            continue;
        };
        // 提示孔那一格由下一轮的 `settle` 收（它非阻塞地拉）；这里只管"是哪位客人的问话孔"。
        if tok != tip
            && let Some(guest) = desk.guest(tok).copied()
        {
            serve_one(&mut board, guest);
        }
        // 三、客人走了 ⇒ 惰性剔（组里那一格随它的孔一起退化，这里只清账）。
        desk.sweep();
    }
}

/// 补齐两件事，返"还有没有没补齐的"。
///
/// 两件事各有各的来源，故各认各的（判据全是确定的号，没有"猜"）：
///
/// - **提示**：装配者推来的客人号（一个号，8 字节）。**非阻塞地拉**——必须在这里拉，
///   不能只在"组唤醒"那一支拉：装配者的推**可能早于本线程把提示孔挂进组**（那一条推
///   落在一个还没有转发登记的站点上），醒不来就得靠这一拉吃到它。提示**在转授之后**，
///   故拉到号就能在同一轮里把答话路认下来（[`reply_of`]）；
/// - **答话路**：装配者转授来的那一枚 ⇒ `admit` 收一位客人（`Taken` = 已经在账上）；
/// - **问话孔**：客人**自己**交来的那一枚 ⇒ 认出来就 `arm` + 挂进组。
fn settle(desk: &mut Desk, assembler: TaskId, tole: &Tole, tip: &mail::HolePie) -> bool {
    // 提示：拉干净（单槽，一位客人一条）。**非阻塞**——它的到达是别人在做的事。
    let mut id = [0u8; 8];
    let mut pending = false;
    while let Ok(8) = tip.pull_timeout(&mut id, 0) {
        let client = TaskId::new(u64::from_le_bytes(id) as usize);
        match reply_of(assembler, client) {
            // `Taken` = 已经在账上（提示是单槽，可能重放）：不换掉原来那位。
            Some(reply) => {
                let _ = desk.admit(client, reply);
            }
            // 次序被破坏（提示先到、答话路不在本表里）：报一句；客人那边会报它自己的超时。
            None => say("board: no reply"),
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

/// 装配者转授来的那一枚答话路（**写端**，落在本表里）。
///
/// 两格判据，都是确定的号：
///
/// - `vestor == assembler` —— **谁转的**。这一格把"客人自己交来的孔"分开：那些的来源位是
///   客人自己（见 [`ask_of`]）；
/// - `owner == who` —— **谁的**。副本共享同一事实（`session` 事实 3），转手不变。
fn reply_of(assembler: TaskId, who: TaskId) -> Option<PieToken> {
    let mut index = 0usize;
    loop {
        let (token, _perm, vestor) = mail::collect(index).ok()?;
        // 越界哨兵：这一遍扫完了。
        if token.get() == 0 {
            return None;
        }
        index += 1;
        if vestor == assembler && bcall::opened_by(token) == Some(who) {
            return Some(token);
        }
    }
}

/// 客人**自己**交来的那一枚问话孔。
///
/// 判据三格，缺一不可：
///
/// - `vestor == who` —— **它亲手交给我的**（与 `register` 同一判据）；
/// - `owner == who` —— 那扇门是它开的（副本共享同一事实）；
/// - **不带 `VEST`** —— 它交来的**入口**（`REGISTER` 的捎带）也满足前两格，而入口是
///   "能再授出"的那一枚（`hang_in` 给了 `VEST`），问话孔只给了读。
fn ask_of(who: TaskId) -> Option<PieToken> {
    let mut index = 0usize;
    loop {
        let (token, perm, vestor) = mail::collect(index).ok()?;
        // 越界哨兵：这一遍扫完了。
        if token.get() == 0 {
            return None;
        }
        index += 1;
        if vestor == who
            && bcall::opened_by(token) == Some(who)
            && !perm.contains(env::Permission::VEST)
        {
            return Some(token);
        }
    }
}

/// 收尾：把本域那枚常驻板线程**点名收掉**。幂等；没起过就无事。
///
/// 会话的收尾由**会话的主人**负责：这枚线程是 root 起的（`attach` 里 `unit::closure`），
/// 也是 root 收的。`attach` 当时 `drop(node)` 弃权、没有 `Join` 可等，故只能按 `HOST`
/// 里那个号点名。
///
/// 用的是既有动词 [`room::doom`]——它的粒度是**域**（"杀它所属的域连同它的子树"），
/// 而这枚线程就住在 root 域里，故这一叫收掉的正是 root 自己那个域：**域亡＝成员清零**。
pub fn shut() {
    let id = HOST.load(Ordering::Acquire);
    if id != 0 {
        let _ = runtime::env::room::doom(TaskId::new(id));
        HOST.store(0, Ordering::Release);
    }
}

/// 板线程的读数：**只在出岔子时说话**（正常一轮什么都不打）。
fn say(msg: &str) {
    let _ = runtime::env::debug::put(msg);
}

/// 招待一位客人：从**它的问话孔**读一帧、交给板、把答话推进**它的答话路**。
///
/// 组已经说了"这一枚有话"，故这一读读得动；期限给 `0` 是**再确认**，不是轮询
/// （单槽的路上不会有两句排队：一位客人一次只问一句）。
fn serve_one(board: &mut Board, guest: protocol::board::Guest) {
    let Some(ask) = guest.ask() else {
        return;
    };
    let mut buf = [0u8; bcall::ASK];
    let Ok(n) = mail::HolePie::from_token(ask).pull_timeout(&mut buf, 0) else {
        return;
    };
    let Some(want) = buf.get(..n) else {
        return;
    };
    // 一问一答：读不懂也答（答 `BAD`），答话走**这位客人的答话路**（一客一路，单槽）。
    let answer = answer(board, want, guest.who());
    let _ = mail::HolePie::from_token(guest.reply()).push(&answer);
}

/// 把一条问交给板，编出一句答（**一格**：读不懂也答，答 `BAD`）。
fn answer(board: &mut Board, want: &[u8], who: TaskId) -> [u8; 1] {
    let Some((op, name, seed)) = bcall::unpack(want) else {
        // 读不懂就答 `BAD`——不猜、不崩。
        return [bcall::BAD];
    };
    let said = match op {
        bcall::REGISTER => match seed {
            // 入口要**是它刚交过来的那一枚**（判据在核心：`probe(entry) == who`）。
            Some(entry) => board.register(name, entry, who).map(|_| ()),
            None => Err(Fail::Denied),
        },
        bcall::UNREGISTER => board.unregister(name, who),
        bcall::LOOKUP => {
            // 查到就**把板上那一份转授给客人**：入口不从报文里走，从会话里走。
            // "查不到"与"授不出去"是两件事，故查的结论优先（`.and`）。
            let mut grant = Ok(());
            board
                .lookup_after(name, |entry| grant = bcall::give(entry, who).map(|_| ()))
                .and(grant)
        }
        // 没见过的动作码：与"这个名字不在板上"同一句话（不另立一格）。
        _ => Err(Fail::Unknown),
    };
    [code(said.err())]
}

/// 失败域 → 答话那一格。
fn code(fail: Option<Fail>) -> u8 {
    match fail {
        None => bcall::OK,
        Some(Fail::Unknown) => bcall::UNKNOWN,
        Some(Fail::Taken) => bcall::TAKEN,
        Some(Fail::Denied) => bcall::DENIED,
        Some(Fail::Full) => bcall::FULL,
    }
}

// ── 客侧（服务域）────────────────────────────────────────────

/// 客侧第一步：装上板那条路，并收下"**答话的是谁**"（[`hear`] 那一格）。
///
/// 返本端这座码头（**答话**从它走，问话走 [`ask_hole`]）与**板线程的号**。
///
/// `holder` = 客人认的对端 = **它的生我者**（孔交给它，它再转授给板线程）——注意它不是板：
/// 客人交出来的孔都落在生我者表里，故"板是谁"得由装配者告诉（见文件头），此后客人交孔、
/// 交入口才叫得出板。
pub fn open(holder: TaskId, ms: usize) -> Result<(Quay, TaskId), Fail> {
    let link = Name::new(LINK).map_err(|_| Fail::Unknown)?;
    let mut quay = Quay::open(holder);
    quay.pair(link, ms).map_err(bcall::map_claim)?;
    let board = hear(&quay, ms).ok_or(Fail::Unknown)?;
    Ok((quay, board))
}

/// 客侧第一步半：铸**问话孔**并交到板手里（本端随即自窄到只写）。
///
/// `board` = [`open`] 收下的那个号。交出去的是可读可写，随后本端 `narrow` 到 `WRITE`：
/// 一条路上只有一个读者（`session` 事实 2），故**板读、本端写**。
///
/// 判"这是问话孔不是入口"的那一格落在板那侧：它不带 `VEST`（入口带）。
pub fn ask_hole(board: TaskId) -> Result<PieToken, Fail> {
    let ask = mail::unseal_hole().map_err(|_| Fail::Denied)?;
    let hole = mail::HolePie::from_token(ask);
    port::ship(&hole, board, Access::READ | Access::WRITE, Policy::NONE)
        .map_err(|_| Fail::Denied)?;
    hole.narrow(env::Permission::WRITE)
        .map_err(|_| Fail::Denied)?;
    Ok(ask)
}

/// 客侧第二步：问一句、取一句答。返答话那一格（[`bcall::OK`] = 板收下了）。
///
/// 问话推 `say`（[`ask_hole`] 铸的那一枚，板读），答话从本端这条板路读（板写）。
///
/// 注册那一句要把**入口**捎上：它经会话交给板（`Accord` 一份），故写进帧里的是"种在板
/// 表里的那个号"——那个号才是板认得的坐标（两个编号空间不同源，互相拿错正是旧树
/// `[33..41]` 那一格的病）。
pub fn ask(
    say: PieToken,
    link: &Quay,
    board: TaskId,
    op: u8,
    name: Name,
    entry: PieToken,
    ms: usize,
) -> Result<u8, Fail> {
    let at = Name::new(LINK).map_err(|_| Fail::Unknown)?;
    let pier = link.find_pier(at).ok_or(Fail::Unknown)?;
    let seed = match op {
        bcall::REGISTER => Some(bcall::hang_in(entry, board).map_err(|()| Fail::Denied)?),
        _ => None,
    };
    // 孔是单槽：槽里还压着上一条时这一推会**等在门外**（`push` 满则挂），不是错误。
    mail::HolePie::from_token(say)
        .push(&bcall::pack(op, name, seed))
        .map_err(|_| Fail::Unknown)?;
    let mut reply = [0u8; 1];
    match pier.pull(&mut reply, ms) {
        Ok(1) => Ok(reply[0]),
        _ => Err(Fail::Unknown),
    }
}

/// 客侧第三步：把**板刚授进来的那一枚**从本端表里取出来（`LOOKUP` 的下场）。
///
/// 一格判据：**来源位是板**（这一份是板交给本端的——`Reply` 里没有号，故只能按"谁给的"认），
/// 取满足的那些里**最后**一枚（表按登记先后枚举；一次一问一答只授一枚，故"最后"就是刚授的）。
///
/// **为什么一格就够**：板只往客人表里送一样东西——查到的入口（它自己不铸码头，见
/// `host_loop`），故"板给的"里没有第二样能与它混。**`owner` 在这里没用**：那扇门是别人开
/// 的（查到谁的入口，开者就是谁），客人不是它的开者。
pub fn take(link: &Quay, board: TaskId) -> Option<PieToken> {
    let at = Name::new(LINK).ok()?;
    let _ = link.find_pier(at)?;
    let mut index = 0usize;
    let mut found = None;
    loop {
        let (token, _perm, vestor) = mail::collect(index).ok()?;
        // 越界哨兵：这一遍扫完了。
        if token.get() == 0 {
            return found;
        }
        index += 1;
        if vestor == board {
            found = Some(token);
        }
    }
}
