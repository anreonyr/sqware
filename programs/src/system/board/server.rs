//! board::server — **板那一台**：编排域里的一枚线程招待所有客人（一枚线程 + 一个组，无轮询）
//!
//! 三侧分家之后本文件只放**板那一台**：编排域里的一枚线程招待所有客人（一枚线程 + 一个组，无轮询）；两侧共用的图与次序说明见 [`super`] 的"载体"那一节，
//! 帧与记号见 [`protocol::system::board::call`]。

use alloc::format;
use env::Mark;

use env::{HoleDir, Name, PieToken, TaskId};
use env::wire::NAME_LEN;
use runtime::core::port::{self, Access, Policy};
use runtime::core::pile::Pile;
use runtime::env::mail;
use runtime::PAGE_SIZE;

use protocol::system::board::call as bcall;
use protocol::system::board::call::ENTRY_MARK;
pub use protocol::system::board::{ASK_MARK, LANE_PREFIX, LINK, TIP_MARK};
use protocol::system::board::{Board, Fail};

use contract::system::board::desk::{Desk, Guest};
use protocol::system::board::call::desk;

/// 还在"补齐两本账"（答话路未认领 / 问话孔未挂上）时，一轮等多久（毫秒）。
///
/// **不是轮询**：账补齐之后这一等就变成 `usize::MAX`（无限等，由组唤醒）；这个短期限只在
/// 装配窗口里用——那几步的到达是**别人**在做（装配者转授、客人自己交孔），板线程没有别的
/// 东西可等。
const SETTLE_MS: usize = 1;

/// 板线程：**一枚线程招待所有客人**。
///
/// ```text
///   起手  铸一枚提示孔（副本交给装配者），把它与所有问话孔挂进**同一个组**
///   循环  补齐两件事（收提示 + 认领答话路 / 认出问话孔并挂组）
///         等一格有事（一个等待）—— 提示孔 ⇒ 来客人了；问话孔 ⇒ 读一帧、答一句
///         说了"我走了"的那一位 ⇒ 撤格 + 摘牌 + 把它的问话孔从组里摘掉
///         惰性剔走**答不出的客人**（`VestedBy` 答 `None`）
/// ```
///
/// **板不用码头**：它要的两枚孔都不是它开的——答话路的**写端**是装配者转授进来的，问话孔的
/// **读端**是客人交进来的，两枚都按 `(谁转的/谁开的, 记号)` 在本表里认出来。铸一枚孔就会给
/// 对端送去一枚用不上的写端（那是一枚"码头"，客人表里的渣），故这里一枚也不铸（提示孔除外：
/// 它的副本正是装配者要的那枚）。
pub(crate) fn host_loop(me: TaskId) {
    // `me` = **装配者**（不是本线程的号）：答话路是它转授来的，故判据的一半是它。
    // 提示孔：本线程铸的那一枚（客人号从这里进来），副本交给装配者。**记号 = `tip`**：
    // 装配者认领那一枚时就按它（它那张表里同时躺着别的路）。
    let Ok(tip) = mail::unseal_hole(TIP_MARK) else {
        say("board: no tip");
        return;
    };
    let tip_hole = mail::HolePie::from_token(tip);
    if port::ship(&tip_hole, me, Access::FETCH | Access::STORE, Policy::VEST).is_err() {
        say("board: tip not handed");
        return;
    }
    // **一个组**：提示孔 + 每位客人的问话孔。提示孔也挂进来，故"来客人了"与"有人问话"
    // 是**同一个等待**——这正是"等 N 位客人说话"要的那一格。本线程独享它（`shared = false`）。
    let Ok(pile) = Pile::unseal(false) else {
        say("board: no group");
        return;
    };
    if pile.attach(&tip_hole, HoleDir::Pull).is_err() {
        say("board: tip not hung");
        return;
    }

    let mut board = bcall::board();
    let mut desk = desk();
    // `who → 死亡道` 的小表：**在 `admit` 那一刻**记——名字随提示那一格来（[`bcall::TIP_LEN`]），
    // 而道按名字认领（[`lane_for`]）。见 [`remember_lane`] / [`take_lane`]。
    let mut lanes: Lanes = [(TaskId::new(0), PieToken::NONE); Desk::CAP];
    let mut swept = 0usize;
    // 收帧的那一页：**在循环外备一次**——每次收帧再备就是一份按帧的分配，正是这一刀要把
    // 它从收帧那一刻拿掉的那件事。载体的界是一页（契约见 `env::fid` 的 `Push`），故一页
    // 装得下任何一条消息；备不下 ⇒ 板起不来（与下面那几处同一个样子）。
    let mut pad: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    if pad.try_reserve_exact(PAGE_SIZE).is_err() {
        say("board: no pad");
        return;
    }
    pad.resize(PAGE_SIZE, 0);
    loop {
        // 一、补齐两件事（收提示 + 认领答话路、认出问话孔并挂组）。还有没补齐的就只等一小段。
        let settling = settle(&mut desk, me, &pile, &tip_hole, &mut lanes);
        // 二、等一格有事。**一个等待**：提示孔或任意一位客人的问话孔。
        let millis = if settling { SETTLE_MS } else { usize::MAX };
        let Ok(Some((tok, _dir))) = pile.await_(millis) else {
            swept += tell_gone(&mut desk, &mut lanes);
            continue;
        };
        // 提示孔那一格由下一轮的 `settle` 收（它非阻塞地拉）；这里只管"是哪位客人的问话孔"。
        if tok != tip
            && let Some(guest) = desk.guest(tok).copied()
        {
            serve_one(&mut board, &mut desk, &pile, guest, swept, &mut lanes, &mut pad);
        }
        // 三、客人**死了**（没道别就没了）⇒ 惰性剔：**那一枚入口答不出**（`VestedBy` 答 `None`
        //     ——不在我表里，**或**它那扇门已经封印）即当场扫空，并推它那条死亡道。
        //     **说了走**的那一位在 `serve_one` 那一支里已经撤干净（撤格 + 摘牌 + 摘孔）。
        swept += tell_gone(&mut desk, &mut lanes);
    }
}

/// 补齐两件事，返"还有没有没补齐的"。
///
/// 两件事各有各的来源，故各认各的（判据全是确定的号，没有"猜"）：
///
/// - **提示**：装配者推来的一位新客人——**号 ＋ 定长名字**（[`bcall::TIP_LEN`]）。**非阻塞地
///   拉**——必须在这里拉，不能只在"组唤醒"那一支拉：装配者的推**可能早于本线程把提示孔挂进
///   组**（那一条推落在一个还没有转发登记的站点上），醒不来就得靠这一拉吃到它。提示**在转授
///   之后**，故拉到就能在同一轮里把答话路认下来（[`reply_of`]）、**并把这一位的死亡道记下**
///   （`who → 道`：道是装配者铸的、名字也是它递来的 ⇒ **客人不必自己报名**）；
/// - **答话路**：装配者转授来的那一枚 ⇒ `admit` 收一位客人（`Taken` = 已经在账上）；
/// - **问话孔**：客人**自己**交来的那一枚 ⇒ 认出来就 `arm` + 挂进组。
fn settle(
    desk: &mut Desk,
    assembler: TaskId,
    pile: &Pile,
    tip: &mail::HolePie,
    lanes: &mut Lanes,
) -> bool {
    // 提示：拉干净（单槽，一位客人一条）。**非阻塞**——它的到达是别人在做的事。
    // 长度不对的那一条**不猜**：`while let` 取不出那一条就收工（与从前那 8 字节的写法同款）。
    let mut rec = [0u8; bcall::TIP_LEN];
    let mut pending = false;
    while let Ok(bcall::TIP_LEN) = tip.pull_timeout(&mut rec, 0) {
        let id = u64::from_le_bytes(rec[..8].try_into().unwrap_or([0u8; 8]));
        let client = TaskId::new(id as usize);
        let name = Name::from_bytes(rec[8..].try_into().unwrap_or([0u8; NAME_LEN])).ok();
        match reply_of(assembler, client) {
            // `Taken` = 已经在账上（提示是单槽，可能重放）：不换掉原来那位。
            Some(reply) => {
                let _ = desk.admit(client, reply);
                // **道就在这一刻认下来**：牌子会被惰性摘掉，摘了就认不出这位叫什么——
                // 而名字刚跟提示一起到（[`lane_for`] 找的正是记号 `gone-<名字>`）。
                if let Some(lane) = name.and_then(lane_for) {
                    remember_lane(lanes, client, lane);
                }
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
                    && pile
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

/// 装配者转授来的那一枚答话路（**写端**，落在本表里）。
///
/// **这枚孔是谁铸的、谁交的**：客人 `seat(板路)` 铸的那一枚（记号 `board`），
/// **经装配者转授**给板线程——所以这一处读的是"交者"。
///
/// 三格判据，都是确定的号：
///
/// - `vestor == assembler` —— **谁转的**。这一格把"客人自己交来的孔"分开：那些的来源位是
///   客人自己（见 [`ask_of`]）；
/// - `owner == who` —— **谁的**：那扇门是**这位客人**开的（副本共享同一事实，转手不变）。
///   这一格不能省：**板招待的是多位客人**，而每位客人那条板路的记号都是 `board`（那是
///   *这条路*的名字）⇒ 只按 `(谁转的, 记号)` 认，几位客人的答话路同形（实测栽过：
///   `router` 与 `guest` 两位在机上，后到的那位认到了前一位的孔）；
/// - **记号 == `board`** —— 那一枚是**板路**上的一枚（客侧 `seat` 铸它时刻的就是这条路
///   的名字 `LINK`；客人自己铸的另两枚刻的是 `ask` / `entry`）。
///
/// **照实记（`Collect` 加宽那一刀）**：本函数与 [`lane_for`] / [`ask_of`] 从前是**每一枚**
/// 都要 `bcall::opened_by` 或 `bcall::marked_as` 各问一次 `Reserve`（一枚一到两次 envcall，
/// 板这一台一轮要扫三遍）；今天那两格随枚举一起回来，判据与哨兵口径**一个字没变**——
/// `owner == who` 在这里等价于 `opened_by(token) == Some(who)`，因为 `who` 是真号、
/// 而"查不出"那一格答 `0`（见 `runtime::env::pie::Pie` 的哨兵口径）。
fn reply_of(assembler: TaskId, who: TaskId) -> Option<PieToken> {
    let board = Mark::of(LINK);
    mail::pies()
        .find(|p| p.vestor == assembler && p.owner == who && p.mark == board)
        .map(|p| p.token)
}

/// 按**名字**认领这一位的死亡道（`gone-<名字>`；装配者铸、转授给本线程）。
///
/// **在 `admit` 那一刻就认**：名字随提示那一格一起来（[`bcall::TIP_LEN`]），而牌子会被惰性
/// 摘掉——等到死亡那一刻再想"它叫什么"就没处问了。名字认不出（名字非法 / 那一条道没转授
/// 过来）⇒ `None`：**这一位死了就没有读数**（与从前"没登记就没读数"同一个静默）。
fn lane_for(name: Name) -> Option<PieToken> {
    let want = Mark::of(&format!("{LANE_PREFIX}{}", name.as_str()));
    mail::pies()
        .find(|p| p.mark == want)
        .map(|p| p.token)
}

/// 本线程的 `who → 死亡道` 小表（一位客人一格；满了就丢——那时板上已经不止 8 位客人）。
type Lanes = [(TaskId, PieToken); Desk::CAP];

/// 把 `who` 的道记下来（同一位置重复登记就覆盖）。
fn remember_lane(lanes: &mut Lanes, who: TaskId, lane: PieToken) {
    for cell in lanes.iter_mut() {
        if cell.0 == who || cell.0.get() == 0 {
            *cell = (who, lane);
            return;
        }
    }
}

/// 取走这一位的道（取走即清：一条道一位客人，一次死亡一份）。
fn take_lane(lanes: &mut Lanes, who: TaskId) -> Option<PieToken> {
    for cell in lanes.iter_mut() {
        if cell.0 == who {
            let lane = cell.1;
            *cell = (TaskId::new(0), PieToken::NONE);
            return Some(lane);
        }
    }
    None
}

/// 剔掉**已经走了**的客人，并把"没了"这件事推进**它那条死亡道**；返剔了几格。
///
/// 判据全在 `Desk::sweep_who` 那一格（`VestedBy` 答 `None`）——**看出来的**那一档。
/// **听来的**那一档（`EVICT`）在 [`answer`] 里推；两档都推，因为装配者只认道。
fn tell_gone(desk: &mut Desk, lanes: &mut Lanes) -> usize {
    let mut dead = [TaskId::new(0); Desk::CAP];
    let n = desk.sweep_who(&mut dead);
    if n == 0 {
        return 0;
    }
    say(&format!("board: swept n={n} occupied={}", desk.occupied()));
    for &who in &dead[..n] {
        if let Some(lane) = take_lane(lanes, who) {
            let _ = mail::HolePie::from_token(lane).push(&[0u8]);
        }
    }
    n
}

/// 这一位客人**自己**交来的那一枚问话孔。
///
/// **这枚孔是谁铸的、谁交的**：客人铸（[`ask_hole`] 刻的记号就是 `ask`）、客人**直接交给板**
/// （不经装配者）——所以这一处读的是"开者"。
///
/// 判据两格，缺一不可：
///
/// - `owner == who` —— 那扇门是它开的（副本共享同一事实）；
/// - **记号 == `ask`** —— 它亲手铸的那一枚问话孔（[`ask_hole`] 刻的）。
///
/// 从前第三格是"**不带 `VEST`**"：它交来的**入口**也满足前两格（交者、开者都是它），而入口
/// 是"能再授出"的那一枚（`ship` 给了 `VEST`）。那一格是**用权限位兼职表达语义**——权限位
/// 回答的是"能不能再授出"，不是"这是什么"，故换成记号：入口刻的是 `entry`（见 [`answer`]
/// 那一支），两枚同来源的孔靠**记号**分开。
fn ask_of(who: TaskId) -> Option<PieToken> {
    let ask = ASK_MARK;
    mail::pies()
        .find(|p| p.owner == who && p.mark == ask)
        .map(|p| p.token)
}

/// 板线程的读数：**只在出岔子时说话**（正常一轮什么都不打）。
fn say(msg: &str) {
    let _ = runtime::env::debug::put(msg);
}

/// 招待一位客人：从**它的问话孔**读一帧、交给板、把答话推进**它的答话路**。
///
/// 组已经说了"这一枚有话"，故这一读读得动；期限给 `0` 是**再确认**，不是轮询
/// （单槽的路上不会有两句排队：一位客人一次只问一句）。
///
/// `pile` 只为一件事进来：客人说了"我走了"之后，**它的问话孔要从组里摘掉**——退场是客人
/// 说的一句，而"不再等这一格"落在组上，故摘孔这一步只能在拿得到组的地方做（`Desk` 那层
/// 够不着组）。
///
/// `buf` = **调用方那一页**（[`host_loop`] 在循环外备一次）：收帧不在这里分配——一页是载体的
/// 界，装得下任何一条消息。
fn serve_one(
    board: &mut Board,
    desk: &mut Desk,
    pile: &Pile,
    guest: Guest,
    swept: usize,
    lanes: &mut Lanes,
    buf: &mut [u8],
) {
    let Some(ask) = guest.ask() else {
        return;
    };
    let Ok(n) = mail::HolePie::from_token(ask).pull_timeout(buf, 0) else {
        return;
    };
    let Some(want) = buf.get(..n) else {
        return;
    };
    // 一问一答：读不懂也答（答 `BAD`），答话走**这位客人的答话路**（一客一路，单槽）。
    let answer = answer(board, desk, want, guest.who(), swept, lanes);
    let _ = mail::HolePie::from_token(guest.reply()).push(&answer);
    // 退场那一句之后：这位客人不会再问了 ⇒ 它的问话孔从组里摘掉（摘完再进下一轮）。
    // **答话先推、摘孔在后**：答话走的是它那条板路（与组无关），次序反了它就收不到 `OK`。
    if bcall::op_of(want) == Some(bcall::EVICT) {
        let _ = pile.detach(&mail::HolePie::from_token(ask), HoleDir::Pull);
    }
}

/// 把一条问交给板，编出一句答（**一格**：读不懂也答，答 `BAD`）。
///
/// **先读动作码、再按码取载荷**：退场那一句是**一字节短帧**（[`bcall::EVICT`]），它没有
/// 名字也没有入口，故在 [`bcall::unpack_ask`] 之前就分流出去——给它塞两格空位是白要 40 字节。
fn answer(
    board: &mut Board,
    desk: &mut Desk,
    want: &[u8],
    who: TaskId,
    swept: usize,
    lanes: &mut Lanes,
) -> [u8; 1] {
    let Some(op) = bcall::op_of(want) else {
        // 读不懂就答 `BAD`——不猜、不崩。
        return [bcall::BAD];
    };
    if op == bcall::EVICT {
        // 死亡道：**先取走**（撤格/摘牌之后就只剩道这一条线索了）。
        let lane = take_lane(lanes, who);
        // 退场：撤它那一格（`Unknown` = **它不在账上**）+ 摘掉它挂在板上的全部牌子。
        let said = match desk.evict(who) {
            Ok(_slot) => {
                let names = board.evict(who);
                // 破例打一行：退场这一件事的读数只此一处（**只在这一件事上打**，不是刷屏）。
                say(&format!(
                    "board: bye tid={} names={names} occupied={} swept={swept}",
                    who.get(),
                    desk.occupied()
                ));
                Ok(())
            }
            Err(fail) => Err(fail),
        };
        // 听来的那一档也要推道：装配者只认道（撤格/摘牌是板自己的账，与它无关）。
        if let Some(lane) = lane {
            let _ = mail::HolePie::from_token(lane).push(&[0u8]);
        }
        return [bcall::fail_to_code(said.err())];
    }
    let Some((name, seed)) = bcall::unpack_ask(want) else {
        return [bcall::BAD];
    };
    let said = match op {
        bcall::REGISTER => match (seed.get() != 0).then_some(seed) {
            // 入口要**是它刚交过来的那一枚**。**这枚孔是谁铸的、谁交的**：客人铸（记号
            // `entry`）、经会话交给板（`ship` ⇒ 板上这一份的来源位是客人）——故判据是
            // `{交者 == 它, 记号 == entry}`：前格在核心（`probe(entry) == who`），后格在这里。
            // 两格缺一不可——它交来的**问话孔**也满足"交者是它"（那一枚也是它铸、它交的），
            // 两件事只有记号分得开。
            Some(entry) if bcall::marked_as(entry) == Some(ENTRY_MARK) => {
                // **登记只管一件事**：把"名字 → 入口"挂到板上（别人据此按名字找得到它）。
                // **死亡道不在这里记**——那一条在 `admit` 那一刻就记下了（名字随提示那一格来、
                // 由装配者递；见 [`bcall::TIP_LEN`] 的照实记）。两件事从此分家：
                // **一位客人不登记也能被监督**（反过来，登记了也不多一条道）。
                board.register(name, entry, who).map(|_| ())
            }
            _ => Err(Fail::Denied),
        },
        bcall::UNREGISTER => board.unregister(name, who),
        bcall::LOOKUP => {
            // 查到就**把板上那一份转授给客人**：入口不从报文里走，从会话里走。
            // "查不到"与"授不出去"是两件事，故查的结论优先（`.and`）。
            let mut grant = Ok(());
            board
                .lookup_after(name, |entry| grant = bcall::ship(entry, who).map(|_| ()))
                .and(grant)
        }
        // 没见过的动作码：与"这个名字不在板上"同一句话（不另立一格）。
        _ => Err(Fail::Unknown),
    };
    [bcall::fail_to_code(said.err())]
}
