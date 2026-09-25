//! board::server — **板那一台**：编排域里的一枚线程招待所有客人（一枚线程 + 一个组，无轮询）
//!
//! 三侧分家之后本文件只放**板那一台**：编排域里的一枚线程招待所有客人（一枚线程 + 一个组，无轮询）；两侧共用的图与次序说明见 [`super`] 的"载体"那一节，
//! 帧与记号见 [`protocol::system::board::call`]。

use env::Wait;
use alloc::format;
use env::Mark;

use env::{HoleDir, Name, PieToken, TaskId};
use runtime::core::port::{self, Access, Policy};
use runtime::core::pile::Pile;
use protocol::session::slip::Slip;
use runtime::env::mail;

use protocol::system::board::call as bcall;
use protocol::system::board::call::ENTRY_MARK;
pub use protocol::system::board::{ASK_MARK, LANE_PREFIX, LINK, TIP_MARK};
use protocol::system::board::{Board, Fail};

use contract::system::desk::{Desk, Guest};
use protocol::system::board::call::desk;

/// 还在"补齐两本账"（答话路未认领 / 问话孔未挂上）时，一轮等多久（毫秒）。
///
/// **不是轮询**：账补齐之后这一等就变成 `Wait::Forever`（由组唤醒）；这个短期限只在
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
    // `who → 死亡道` 的小表：**在 `admit` 那一刻**记——名字随提示那一格来（[`bcall::Tip::LEN`]），
    // 而道按名字认领（[`lane_for`]）。见 [`remember_lane`] / [`take_lane`]。
    let mut lanes: Lanes = alloc::vec::Vec::new();
    let mut swept = 0usize;
    // **收帧的那一页**：在循环外备一次。门的缓冲是**载体的一页**，不是家族帧那么大——
    // 见 [`Slip::land_in`]：客人推得进来、比这一族最长那一枚更长的一条也得**取得出来**
    // （读不懂就答 `BAD`），否则它永远留在槽里（取不出 ⇒ 槽原样），这道门从此卡死且空转。
    //
    // **照实记（刀一那一句被证伪）**：本处原写着"缓冲躺在船台自己身上，尺寸就是这一族最长
    // 那一枚——那一页不再需要"。那是假的：`land` 拿家族帧那只缓冲收不下更长的推，而核**不丢**
    // 取不出的那一条。判据是 `harness/src/probe_bound.rs` 第四条（板那一道门那一腿）。
    let mut buf: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    if buf.try_reserve_exact(runtime::PAGE_SIZE).is_err() {
        say("board: no room");
        return;
    }
    buf.resize(runtime::PAGE_SIZE, 0);
    loop {
        // 一、补齐两件事（收提示 + 认领答话路、认出问话孔并挂组）。还有没补齐的就只等一小段。
        let settling = settle(&mut desk, &pile, &tip_hole, &mut lanes);
        // 二、等一格有事。**一个等待**：提示孔或任意一位客人的问话孔。
        let millis = if settling { Wait::AtMost(SETTLE_MS) } else { Wait::Forever };
        let Ok(Some((tok, _dir))) = pile.await_(millis) else {
            swept += tell_gone(&mut desk, &mut lanes);
            continue;
        };
        // 提示孔那一格由下一轮的 `settle` 收（它非阻塞地拉）；这里只管"是哪位客人的问话孔"。
        if tok != tip
            && let Some(guest) = desk.guest(tok).copied()
        {
            serve_one(
                &mut board,
                &mut desk,
                &pile,
                guest,
                swept,
                &mut lanes,
                &mut buf,
            );
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
/// - **提示**：装配者推来的一位新客人——**号 ＋ 定长名字 ＋ 答话路那一格**
///   （[`bcall::Tip::LEN`]）。**非阻塞地拉**——必须在这里拉，不能只在"组唤醒"那一支拉：装配者的推**可能早于本线程把提示孔挂进
///   组**（那一条推落在一个还没有转发登记的站点上），醒不来就得靠这一拉吃到它。提示**在转授
///   之后**，故拉到就能在同一轮里把答话路认下来（**末格就在这条记录里**）、**并把这一位的死亡道记下**
///   （`who → 道`：道是装配者铸的、名字也是它递来的 ⇒ **客人不必自己报名**）；
/// - **答话路**：那一格随提示一起来（装配者转授时把 `to.seed()` 编进这条记录），板拿它
///   **一次 `Reserve`** 验过 ⇒ `admit` 收一位客人（`Taken` = 已经在账上）。判据与旧那一扫
///   一字不差（开者 = 这位客人 ＋ 记号 = 板路），只是不再扫自己的表；
/// - **问话孔**：客人**自己**交来的那一枚 ⇒ 认出来就 `arm` + 挂进组。
fn settle(
    desk: &mut Desk,
    pile: &Pile,
    tip: &mail::HolePie,
    lanes: &mut Lanes,
) -> bool {
    // 提示：拉干净（单槽，一位客人一条）。**非阻塞**——它的到达是别人在做的事。
    // 长度不对的那一条**不猜**：`while let` 取不出那一条就收工（与从前那 8 字节的写法同款）。
    let mut rec = [0u8; bcall::Tip::LEN];
        let mut pending = false;
        let board = Mark::of(LINK);
        while let Ok(bcall::Tip::LEN) = tip.pull_timeout(&mut rec, Wait::POLL) {
            // **帧形只有一处**：三格怎么切全在 [`bcall::Tip`] 那一对里。从前这里三行手切，
            // 其中一句注释自认"最容易切错"（"名字按自己的宽度切……静默退回空名"）——它随这一刀
            // 一起退场。**代价照实**：名字那一段读不成一个合法 `Name` 时，从前是"当没有名字、
            // 照样收下这位客人"，现在是**整帧读不懂**（答一句 `board: no reply`，不收）——
            // 那一格只有装配者推得进来，而它推的一定是合法名字。
            let Some(tip) = bcall::Tip::fetch(&rec) else {
                say("board: no reply");
                continue;
            };
            let client = tip.who;
            // 答话路那一格随提示一起来：**一次 `Reserve` 验它**，不扫自己的表。
            match mail::reserve(tip.reply) {
                Ok((_vestor, owner, mark)) if owner == client && mark == board => {
                    let _ = desk.admit(client, tip.reply);
                    // **道就在这一刻认下来**：牌子会被惰性摘掉，摘了就认不出这位叫什么——
                    // 而名字刚跟提示一起到（[`lane_for`] 找的正是记号 `gone-<名字>`）。
                    if let Some(lane) = lane_for(tip.name) {
                        remember_lane(lanes, client, lane);
                    }
                }
                // 次序被破坏（提示先到、答话路不在本表里 / 那一格指的不是这一位）：报一句；
                // 客人那边会报它自己的超时。
                _ => say("board: no reply"),
            }
        }

    // 还没挂上问话孔的那几格：**账自己按格子号走一遍**（见 [`Desk::arm_pending`]）——
    // 调用方这一侧因此不必先抄一份到自己的栈上，那一张按常数开的数组就此退场。
    pending |= desk.arm_pending(
        |who| ask_of(who),
        |ask| {
            pile.attach(&mail::HolePie::from_token(ask), HoleDir::Pull)
                .is_ok()
        },
    );
    pending
}

/// 按**名字**认领这一位的死亡道（`gone-<名字>`；装配者铸、转授给本线程）。
///
/// **在 `admit` 那一刻就认**：名字随提示那一格一起来（[`bcall::Tip::LEN`]），而牌子会被惰性
/// 摘掉——等到死亡那一刻再想"它叫什么"就没处问了。名字认不出（名字非法 / 那一条道没转授
/// 过来）⇒ `None`：**这一位死了就没有读数**（与从前"没登记就没读数"同一个静默）。
fn lane_for(name: Name) -> Option<PieToken> {
    let want = Mark::of(&format!("{LANE_PREFIX}{}", name.as_str()));
    mail::pies()
        .find(|p| p.mark == want)
        .map(|p| p.token)
}

/// 本线程的 `who → 死亡道` 小表（一位客人一格；**备不下就丢这一条读数**）。
///
/// **照实记（它为什么是 `Vec`）**：它原先是 `[(TaskId, PieToken); Desk::CAP]`——**借来的界**
/// （客人账那个常数）。两本客人账并成一本、常数退场之后那个界就没了，而这里本来就有"满了就
/// 丢"的下场 ⇒ 如实收成 `Vec` + 失败即丢。丢的是一条**死亡读数**，不是监督本身：牌子由
/// 板自己扫，道只喂装配者。
type Lanes = alloc::vec::Vec<(TaskId, PieToken)>;

/// 把 `who` 的道记下来（同一位重复登记就覆盖）；**备不下就丢**（见上面那一格）。
fn remember_lane(lanes: &mut Lanes, who: TaskId, lane: PieToken) {
    if let Some(cell) = lanes.iter_mut().find(|cell| cell.0 == who) {
        cell.1 = lane;
        return;
    }
    if lanes.try_reserve(1).is_ok() {
        lanes.push((who, lane));
    }
}

/// 取走这一位的道（取走即清：一条道一位客人，一次死亡一份）。
fn take_lane(lanes: &mut Lanes, who: TaskId) -> Option<PieToken> {
    let at = lanes.iter().position(|cell| cell.0 == who)?;
    Some(lanes.remove(at).1)
}

/// 剔掉**已经走了**的客人，并把"没了"这件事推进**它那条死亡道**；返剔了几格。
///
/// 判据全在 [`Desk::sweep_each`] 那一格（`VestedBy` 答 `None`）——**看出来的**那一档。
/// **听来的**那一档（`EVICT`）在 [`answer`] 里推；两档都推，因为装配者只认道。
/// **照实记（两个副作用的次序换了）**：从前是"先打一行、再挨个推道"（号先抄进一个按常数
/// 开的 `dead` 缓冲）。现在号只在回调里拿得到，故推道在打行之前。两条读数走两条不同的路
/// （道是消息、`say` 是串口），这个次序不承载意义。
fn tell_gone(desk: &mut Desk, lanes: &mut Lanes) -> usize {
    let n = desk.sweep_each(|who| {
        if let Some(lane) = take_lane(lanes, who) {
            let _ = mail::HolePie::from_token(lane).push(&[0u8]);
        }
    });
    if n > 0 {
        say(&format!("board: swept n={n} occupied={}", desk.occupied()));
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
    // 一问一答：读不懂也答（答 `BAD`），答话走**这位客人的答话路**（一客一路，单槽）。
    // **解码只做一次**：答哪一句由它定，下面"要不要摘掉它那枚问话孔"也由它定。
    //
    // **照实记（`buf` 那一页为什么回来）**：收帧用的是**载体那一页**（调用方在循环外备的那
    // 一只），不是船台自己那只家族缓冲——理由与实测见 [`Slip::land_in`] 的照实记：拿家族缓冲
    // 收不下比它更长的一条，核**不丢**取不出的消息 ⇒ 门卡死且空转。
    let decoded = Slip::<bcall::Req>::seal(ask).land_in(buf, Wait::POLL);
    let said = match decoded {
        Some(ask) => answer(board, desk, ask, guest.who(), swept, lanes),
        // 空帧 / 长度不对：读不懂就答 `BAD`——不猜、不崩。
        None => bcall::BAD,
    };
    // 答一句：**一格**（[`bcall::Rep`] 那一张形状）——装与发都不在这一层写字节。
    let _ = Slip::<bcall::Rep>::seal(guest.reply())
        .load(bcall::Rep::of(said))
        .ship();
    // 退场那一句之后：这位客人不会再问了 ⇒ 它的问话孔从组里摘掉（摘完再进下一轮）。
    // **答话先推、摘孔在后**：答话走的是它那条板路（与组无关），次序反了它就收不到 `OK`。
    if matches!(decoded, Some(bcall::Wire::Evict)) {
        let _ = pile.detach(&mail::HolePie::from_token(ask), HoleDir::Pull);
    }
}

/// 把一条问交给板，编出一句答（**一格**：读不懂也答，答 `BAD`）。
///
/// **形状由 [`bcall::Wire`] 说**：退场那一句是**一字节短帧**（没有名字也没有入口），
/// 另外三条各按自己那份荷载走；表外的动作码是**单独一格**（它不是"读不懂"，答的话也不同）。
///
/// **照实记（`op_of` 那一手退场）**：从前这里先 `op_of` 读裸码、按码分流、再 `match op` 认
/// 四枚 `u8`——两个 `match` 都有"没见过的码"那一支，多一条动作**编得过**。现在那个 `match`
/// 是穷举的：少写一条动作，这里当场编不过。
fn answer(
    board: &mut Board,
    desk: &mut Desk,
    ask: bcall::Wire,
    who: TaskId,
    swept: usize,
    lanes: &mut Lanes,
) -> u8 {
    let said = match ask {
        bcall::Wire::Evict => {
            // 死亡道：**先取走**（撤格/摘牌之后就只剩道这一条线索了）。
            let lane = take_lane(lanes, who);
            // 退场：撤它那一格（`None` = **它不在账上**）+ 摘掉它挂在板上的全部牌子。
            let said = match desk.evict(who) {
                Some(_slot) => {
                    let names = board.evict(who);
                    // 破例打一行：退场这一件事的读数只此一处（**只在这一件事上打**，不是刷屏）。
                    say(&format!(
                        "board: bye tid={} names={names} occupied={} swept={swept}",
                        who.get(),
                        desk.occupied()
                    ));
                    Ok(())
                }
                None => Err(contract::system::board::core::Fail::Unknown),
            };
            // 听来的那一档也要推道：装配者只认道（撤格/摘牌是板自己的账，与它无关）。
            if let Some(lane) = lane {
                let _ = mail::HolePie::from_token(lane).push(&[0u8]);
            }
            return bcall::fail_to_code(said.err());
        }
        bcall::Wire::Register { name, seed } => match (seed.get() != 0).then_some(seed) {
            // 入口要**是它刚交过来的那一枚**。**这枚孔是谁铸的、谁交的**：客人铸（记号
            // `entry`）、经会话交给板（`ship` ⇒ 板上这一份的来源位是客人）——故判据是
            // `{交者 == 它, 记号 == entry}`：前格在核心（`probe(entry) == who`），后格在这里。
            // 两格缺一不可——它交来的**问话孔**也满足"交者是它"（那一枚也是它铸、它交的），
            // 两件事只有记号分得开。
            //
            // **那一格 0 是"这枚号不合法"，不是"这一码没带"**：动作码那一格已经说了它会带
            // （见 [`bcall::Req`]），故 0 只剩一个意思——**这一帧不是我们的客人编的**。
            Some(entry) if bcall::marked_as(entry) == Some(ENTRY_MARK) => {
                // **登记只管一件事**：把"名字 → 入口"挂到板上（别人据此按名字找得到它）。
                // **死亡道不在这里记**——那一条在 `admit` 那一刻就记下了（名字随提示那一格来、
                // 由装配者递；见 [`bcall::Tip::LEN`] 的照实记）。两件事从此分家：
                // **一位客人不登记也能被监督**（反过来，登记了也不多一条道）。
                board.register(name, entry, who).map(|_| ())
            }
            _ => Err(Fail::Denied),
        },
        bcall::Wire::Unregister { name } => board.unregister(name, who),
        bcall::Wire::Lookup { name } => {
            // 查到就**把板上那一份转授给客人**：入口不从报文里走，从会话里走。
            // "查不到"与"授不出去"是两件事，故查的结论优先（`.and`）。
            let mut grant = Ok(());
            board
                .lookup_after(name, |entry| grant = bcall::ship(entry, who).map(|_| ()))
                .and(grant)
        }
        // 没见过的动作码：与"这个名字不在板上"同一句话（不另立一格）。
        bcall::Wire::Unknown => Err(Fail::Unknown),
    };
    bcall::fail_to_code(said.err())
}
