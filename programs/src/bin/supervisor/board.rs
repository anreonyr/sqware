//! board — **板那两半**：板侧（[`attach`] 起板线程 + 转授），客侧（[`open`] / [`ask`] / [`take`]）。
//!
//! 板是**装配者那个域里的一枚线程**（就一枚，招待所有客人），客人是别的域里的线程。
//! 两侧各用会话的一侧（[`Quay::pair`] 的两半）：
//!
//! ```text
//!   装配者（root）                            客人（服务域）            板线程（一枚）
//!   quay.seat(名字) + quay.claim(客人) ───▶  pair(名字)  ── 交一枚孔 ──▶  seat(名字)
//!     得到客人那一枚（客人读答话）              得到板那一枚（客人写问话）   claim(客人) 认出写端
//!   转授：把客人那一枚 Ship 给板线程 ─────────────────────────────────▶  写端到手
//!   提示：往提示之路推一个客人号 ────────────────────────────────────▶  给它开一座码头
//!                                             post(Query) ──────────────▶  pull ⇒ 交给板
//!                                             pull(Reply) ◀──────────────  post(Reply)
//! ```
//!
//! **为什么中间要装配者过一手**：客人只认得它的生我者（孔是交给"生我者"的），板线程不是
//! 它的生我者 ⇒ 客人交出来的那一枚落在装配者表里。装配者把它**转授**给板线程——装配者本来
//! 就是设备门闩的第一个持有者与转授者，这里走的是同一条路。转授的是**客人开的那扇门**：
//! 副本共享 `owner`（`session` 事实 3），故板那一侧"按 `owner` 认领"照样认得出它。
//!
//! # 板为什么就一枚线程
//!
//! 牌子上的入口是**一枚只在对端表里有意义的句柄**（`PieToken` = "我这张表里的第几个"）。
//! "查到了要把入口授出去"必须由**持有那一枚的那张表**来做——故所有牌子只能住同一张表，
//! 也就是同一枚线程。先前"一位客人一枚待客线程"的形状正是栽在这里：甲的入口在甲的板线程
//! 表里，乙来查时乙的板线程手里没有它 ⇒ `probe` 判它"已死"（牌子当场被扫空）、`give` 也
//! 交不出去。**症状是"刚挂上的名字，别人一查就是 `Unknown`"**。
//!
//! 一枚线程招待多位客人靠**轮询**：门铃与孔各有各的唤醒键，今天没有"同时等 N 个源"。
//! 每处 `pull` 都带期限（真的挂起，不是空转），一圈 = 每位客人 [`POLL_MS`]。
//!
//! # 一问一答的次序
//!
//! ```text
//!   1  装配者在本域与客人那座码头上加一条板泊位（本端交出自己那一枚 —— 客人往它写问话）
//!   2  认领：**这位客人**交出来的那一枚落进本域表里（客人只认得生我者，故先落这里）
//!   3  板线程（只起一枚）→ 提示它来客人了 → 把客人那一枚转授给它
//! ```
//!
//! 三步都在 [`attach`] 里，**次序即契约**：第 3 步之前板线程还没起（起了也无用——它等的
//! 就是这一枚），而客人那一枚要到第 2 步才在。
//!
//! # 字节长什么样
//!
//! 帧形、三个动作码、一处上界、答话那一格全在 [`protocol::board::call`]；本文件只做
//! "读一条 → 交给板 → 回一句"，一个字节都不自己编。

use core::sync::atomic::{AtomicUsize, Ordering};

use env::{Name, PieToken, TaskId};
use protocol::board::call as bcall;
use protocol::board::{Board, Fail};
use protocol::session::Quay;
use runtime::core::port::{self, Access, Policy};
use runtime::core::unit::{self, Join};
use runtime::env::mail;

/// 板那条通道的名字：**两侧同一个**（泊位自己的坐标，不进报文）。
pub const LINK: &str = "board";

/// 提示之路的名字（只有装配者那侧用得上：板线程那一枚是它自己铸的，不需要名字）。
const TIP_NAME: &str = "board-tip";

/// 板线程一轮里，一位客人最多等多久（毫秒）——**轮询的步长**。
///
/// 一位 1ms：客人再多，一圈也只是一圈毫秒；闲时板线程在 `wait` 里真挂起（孔有唤醒），
/// 不是空转。
const POLL_MS: usize = 1;

/// 板线程手里最多几座码头（一位客人一座）。条数是策略，容器要有界。
const GUESTS: usize = 8;

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
    // 1. 本端那一枚交出去（落在本域表里——客人拿不到它，也不需要：客人往板那一枚写问话）。
    quay.seat(link, ms).map_err(|_| "board:seat")?;
    // 2. 认领**这位客人**交出来的那一枚。本域给每个孩子各开一座码头，故认的是"它给我的"，
    //    不是"我没开过的"——不然会把别的客人的孔配到它头上（`Quay::claim` 的正文）。
    //    **次序**：板那条比 `records` 后到，而 `records` 的写端已经用掉了 ⇒ 这一枚认在
    //    板泊位上（一枚孔只配一条泊位）。
    quay.claim(client, ms).map_err(|_| "board:claim")?;
    // 3. 板线程（只起一枚）→ 告诉它来客人了 → 把客人那一枚转授过去。
    let host = host(me, ms, tip)?;
    let Some(tip) = *tip else {
        return Err("board:tip");
    };
    tell(client, tip).map_err(|_| "board:tell")?;
    hand(quay, host).map_err(|()| "board:hand")
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

/// 告诉板线程：来客人了（一个号，8 字节）。**满则等**——那是单槽，板线程一轮就读走。
fn tell(client: TaskId, tip: PieToken) -> Result<(), ()> {
    let tip = mail::HolePie::from_token(tip);
    tip.push(&(client.get() as u64).to_le_bytes())
        .map_err(|_| ())
}

/// 把**客人交出来的那一枚**转授给板线程。
///
/// 转授的是"客人开的那扇门"（`owner` 是客人），板那侧认领时认的正是它。
///
/// 子集只给 `R|W`，**不加 `VEST`**：板线程用这一枚读写答话，不需要再授出——一分不多。
/// 本域自己那一份转授之后**不收**：内核那侧"交出"（`CAGE`）要求源枚自己带 `CAGE`，而客人
/// 给过来的这一枚没有（`seat` 给的是 `R|W|VEST`）；收它要多一条 `release`，而这一步之后
/// 没有任何东西再碰它——本域常驻，随域退场一起回收。
fn hand(quay: &Quay, host: TaskId) -> Result<(), ()> {
    let link = Name::new(LINK).map_err(|_| ())?;
    let pier = quay.find(link).ok_or(())?;
    let hole = mail::HolePie::from_token(pier.at_peer());
    port::ship(&hole, host, Access::READ | Access::WRITE, Policy::NONE)
        .map(|_| ())
        .map_err(|_| ())
}

/// 板线程：**一枚线程招待所有客人**。
///
/// ```text
///   起手  铸一枚提示孔，副本交给装配者
///   循环  读提示（有就来一位客人 ⇒ 给它开一座码头、装一条板路）
///         轮着看每位客人那条路（一位最多等 POLL_MS），有问就答、答话往同一枚推
/// ```
fn host_loop(me: TaskId) {
    // 提示孔：本线程铸的那一枚（客人号从这里进来），副本交给装配者。
    let Ok(tip) = mail::unseal_hole() else {
        say("board: no tip");
        return;
    };
    let hole = mail::HolePie::from_token(tip);
    if port::ship(&hole, me, Access::READ | Access::WRITE, Policy::VEST).is_err() {
        say("board: tip not handed");
        return;
    }

    let mut board = bcall::board();
    let mut guests: [Option<Quay>; GUESTS] = [const { None }; GUESTS];
    loop {
        // 一、提示：来客人了 ⇒ 给它开一座码头（`seat` 就是"本线程那一枚交给它"）。
        //    **没有客人时这一等就是唯一的挂起处**（不为空转烧核）；有客人时下面的轮询
        //    自己会等，读提示就不必再等。
        let wait = if guests.iter().all(Option::is_none) {
            POLL_MS
        } else {
            0
        };
        let mut id = [0u8; 8];
        if let Ok(8) = hole.pull_timeout(&mut id, wait) {
            let client = TaskId::new(u64::from_le_bytes(id) as usize);
            if open_guest(&mut guests, client).is_err() {
                say("board: no seat");
            }
        }
        // 二、轮着看每位客人：有问就答（没问到就挂起 POLL_MS）。
        for quay in guests.iter_mut().flatten() {
            serve_one(&mut board, quay);
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

/// 来了一位客人：给它开一座码头，并在那座码头上装一条板路。
fn open_guest(guests: &mut [Option<Quay>; GUESTS], client: TaskId) -> Result<(), ()> {
    let link = Name::new(LINK).map_err(|_| ())?;
    let Some(slot) = guests.iter_mut().find(|g| g.is_none()) else {
        // 名额用完：这位客人接不上了（装配那边等不到板那一枚，会报它自己的装配号）。
        return Err(());
    };
    let mut quay = Quay::open(client);
    quay.seat(link, 0).map_err(|_| ())?;
    *slot = Some(quay);
    Ok(())
}

/// 招待一位客人一轮：先认一遍（装配者转授过来的那一枚随时可能到），再看有没有问。
fn serve_one(board: &mut Board, quay: &mut Quay) {
    let Ok(link) = Name::new(LINK) else {
        return;
    };
    // 认领**这位客人**交出来的那一枚（`owner` = 客人）：转授可能比这一轮晚到，故每轮都试。
    if let Some(client) = quay.peer() {
        let _ = quay.claim(client, 0);
    }
    let Some(pier) = quay.find_pier(link) else {
        return;
    };
    let mut buf = [0u8; bcall::ASK];
    let Ok(n) = pier.pull(&mut buf, POLL_MS) else {
        return;
    };
    let Some(want) = buf.get(..n) else {
        return;
    };
    // 一问一答：读不懂也答（答 `BAD`），答话走**同一枚孔**（单槽，交替）。
    let answer = answer(board, want, pier.peer());
    let _ = pier.post(&answer);
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

/// 客侧第一步：装上板那条路。返本端这座码头——问与答都从它走。
///
/// `holder` = 客人认的对端 = **它的生我者**（孔交给它，它再转授给板线程）。
pub fn open(holder: TaskId, ms: usize) -> Result<Quay, Fail> {
    let link = Name::new(LINK).map_err(|_| Fail::Unknown)?;
    let mut quay = Quay::open(holder);
    quay.pair(link, ms).map_err(bcall::map_claim)?;
    Ok(quay)
}

/// 客侧第二步：问一句、取一句答。返答话那一格（[`bcall::OK`] = 板收下了）。
///
/// 注册那一句要把**入口**捎上：它经会话交给板（`Accord` 一份），故写进帧里的是"种在板
/// 表里的那个号"——那个号才是板认得的坐标（两个编号空间不同源，互相拿错正是旧树
/// `[33..41]` 那一格的病）。
pub fn ask(link: &Quay, op: u8, name: Name, entry: PieToken, ms: usize) -> Result<u8, Fail> {
    let at = Name::new(LINK).map_err(|_| Fail::Unknown)?;
    let pier = link.find_pier(at).ok_or(Fail::Unknown)?;
    let seed = match op {
        bcall::REGISTER => {
            // 板是谁**从孔本身读**：客人手里那一格是生我者，不是板（板是转授过去的那位）。
            let board = bcall::peer_of(&pier).ok_or(Fail::Unknown)?;
            Some(bcall::hang_in(entry, board).map_err(|()| Fail::Denied)?)
        }
        _ => None,
    };
    // 孔是单槽：槽里还压着上一条时这一推会**等在门外**（`push` 满则挂），不是错误。
    pier.post(&bcall::pack(op, name, seed))
        .map_err(|()| Fail::Unknown)?;
    let mut reply = [0u8; 1];
    match pier.pull(&mut reply, ms) {
        Ok(1) => Ok(reply[0]),
        _ => Err(Fail::Unknown),
    }
}

/// 客侧第三步：把**板刚授进来的那一枚**从本端表里取出来（`LOOKUP` 的下场）。
///
/// 三格判据，缺一不可：
///
/// - `vestor` = **板**（这一份是板交给本端的——`Reply` 里没有号，故只能按"谁给的"认）；
/// - **不是本端这条板路自己那两枚**（本端铸的读端、板交来的写端——**后者的 `vestor` 也是
///   板**，不排除就会认成"授进来的那一枚"）；
/// - 取表里**最后**那一枚（表按登记先后枚举；一次一问一答只授一枚，故"最后"就是刚授的）。
///
/// **`owner` 在这里没用**：那扇门是**对端**开的，客人不是它的开者。按 `owner` 找只在
/// "挂的是自己铸的那一枚"时碰巧成立（自重登、自问自答的那一趟）——换一位真客人就找不到。
pub fn take(link: &Quay) -> Option<PieToken> {
    let at = Name::new(LINK).ok()?;
    let pier = link.find_pier(at)?;
    let board = bcall::peer_of(&pier)?;
    let mine = [pier.hole(), pier.at_peer()];
    let mut index = 0usize;
    let mut found = None;
    loop {
        let (token, _perm, vestor) = mail::collect(index).ok()?;
        // 越界哨兵：这一遍扫完了。
        if token.get() == 0 {
            return found;
        }
        index += 1;
        if vestor == board && !mine.contains(&token) {
            found = Some(token);
        }
    }
}
