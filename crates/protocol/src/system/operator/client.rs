//! operator::client — **客侧**：「持树者是谁」由装配侧递一格，此后一问一答
//!
//! 三侧分家之后本文件只放**客侧**：「持树者是谁」由装配侧递一格，此后一问一答；两侧共用的图
//! 与说明见 [`super`] 的"载体"那一节，帧与记号见 [`crate::system::operator`]。
//!
//! **一手对一条原语**（`land` / `part` / `find` / `trim` / `list` / `seek` / `name`）：线上与模型
//! 是同一件事的两层，客侧这一层也不再拿一个 `op` 码当参数——问什么形状由函数名说。

use contract::message::Message;
use env::wire::Field;
use env::Wait;
use env::Mark;
use env::{Name, PieToken, TaskId};
use runtime::core::port::{self, Access, Policy};
use runtime::env::mail::{self, AnyPie};

use crate::system::operator::Fail;
use crate::system::operator as ocall;
use crate::system::operator::core::judge::Id;
use crate::system::operator::core::judge::Rule;
pub use crate::system::operator::{ASK_MARK, LINK, TIP_NAME};
use crate::system::operator::{EntryId, Listing, Where};
use crate::session::Quay;
use crate::session::slip::Slip;

/// 客侧第一步：装上树那条路（**记号就是这条路的名字**），认下对端那一枚，并收下
/// "**答话的是谁**"（[`hear`] 那一格）。
///
/// 返本端这座码头（**答话**从它走，问话走 [`ask_hole`]）与**持树者的号**。
///
/// `holder` = 客人认的对端 = **它的生我者**（孔交给它，它再转授给持树者）——注意它不是
/// 持树者：客人交出来的孔都落在生我者表里，故"持树者是谁"得由装配者告诉（见文件头）。
pub fn open(holder: TaskId, millis: Wait) -> Result<(Quay, TaskId), Fail> {
    let link = Name::new(LINK).map_err(|_| Fail::Unknown)?;
    let mut quay = Quay::open(holder, crate::session::call::hands());
    quay.seat(link).map_err(ocall::core::map_seat)?;
    quay.claim(holder, Mark::of(link.as_str()), millis)
        .map_err(ocall::core::map_claim)?;
    let host = hear(&quay, millis).ok_or(Fail::Unknown)?;
    Ok((quay, host))
}

/// 客侧第一步半：铸**问话孔**并交到持树者手里（本端随即自窄到只写）。
///
/// `host` = [`open`] 收下的那个号。交出去的是可读可写，随后本端 `narrow` 到 `STORE`：
/// 一条路上只有一个读者，故**持树者读、本端写**。
///
/// 记号 = [`ASK_MARK`]：持树者那侧就是按它把这枚孔与**入口**分开的（两枚都由本端铸、本端交）。
pub fn ask_hole(host: TaskId) -> Result<PieToken, Fail> {
    // **一个域只铸一枚问话孔**——先找我表里那一枚，有就不铸第二枚。
    //
    // 认的是"**本端开的** + 记号"两格（[`crate::session::call::find`]，与持树者那一侧认孔
    // 的两格正判据同一句话）。于是"一个域只铸一枚"从**纪律**变成**构造**：这一条路再也生不出
    // 第二枚，而第二枚的症状是"多出来的那枚永远没人读它的推"（`server.rs::claim` 那一格记着）。
    //
    // 照实记：这一手**不改签名、不动调用点**——`ASK_MARK` 全仓只有本函数用它铸孔，
    // 故把闸装在这个记号唯一的铸者身上，与"把它改成有名有姓的泊位（`Quay::seat`）"是**同一句
    // 保证的小形状**（那一版要改二十处调用点的签名）。
    if let Some(have) = crate::session::call::find(me()?, ASK_MARK) {
        return Ok(have);
    }
    let ask = mail::unseal_hole(ASK_MARK).map_err(|_| Fail::Unknown)?;
    let hole = mail::HolePie::from_token(ask);
    port::ship(&hole, host, Access::FETCH | Access::STORE, Policy::NONE)
        .map_err(|_| Fail::Unknown)?;
    hole.narrow(env::Permission::STORE)
        .map_err(|_| Fail::Unknown)?;
    Ok(ask)
}

/// **本端是哪一枚线程**——"这一枚孔是谁开的"那一问要它。
///
/// 认领的两格正判据里，"谁开的"是内核盖的那个戳；而**铸孔的人**就是本线程 ⇒ 扫自己这张表
/// 时它只能是自己。
fn me() -> Result<TaskId, Fail> {
    runtime::env::unit::self_id().map_err(|_| Fail::Unknown)
}

/// 客侧第二步（内里那一手）：**编好的一问推上去，收一句答**。
///
/// 问话推 `say`（[`ask_hole`] 铸的那一枚，持树者读），答话从本端这条树路读（持树者写）。
/// 返**收进来的那一答**（[`ocall::Said`]）——**形状由问的人自己读**（答的四种形状在线上分不开，
/// 见 `Said` 的照实记：他问的是哪一条，他自己知道）。
fn ask_out(
    say: PieToken,
    link: &Quay,
    ask: ocall::Req<'_>,
    millis: Wait,
) -> Result<ocall::Said, Fail> {
    let at = Name::new(LINK).map_err(|_| Fail::Unknown)?;
    let pier = link.find(at).ok_or(Fail::Unknown)?;
    // 发：装上、发出去——**一帧＝一条报**（偏移与长度不在这层：字段表与 `Message` 说）。
    // 孔是单槽：槽里还压着上一条时这一推会**等在门外**（`push` 满则挂），不是错误。
    Slip::<ocall::Req<'_>>::seal(say)
        .load(ask)
        .ship()
        .map_err(|_| Fail::Unknown)?;
    // 收：答话走本端这条树路——与板那一族同一个形状（`Slip::<Union>::seal(pier.hole()).land(buf, ..)`）。
    // 缓冲由调用方给：这条树路只有持树者会写 ⇒ 本族那只空缓冲（[`Message::EMPTY`]）就够。
    let mut buf = ocall::Union::EMPTY;
    Slip::<ocall::Union>::seal(pier.hole())
        .land(buf.as_mut(), millis)
        // 两格失败（没收到 / 解不动）在这一侧落同一格：对本端是同一个下一步。
        .map_err(|_| Fail::Unknown)
}

/// 客侧第二步（**落**）：在 `at` 那一块 `Pane` 里给 `name` 贴一枚 `Tile`；答**那一格自己的号**。
///
/// `entry` 是客人手里那一枚：它**经会话交给持树者**（`Accord` 一份）后才进帧——报文里走的
/// 是"种在持树者表里的那个号"，那才是它认得的坐标（见文件头与 [`ocall::ship`]）。
///
/// `rule` / `mine` 是**这一格的两轴条件**（用 / 改）——落牌的人当场声明，此后就由持树者
/// 那一本账替它记着；默认是"公开 + 不声明归属"（既有的装配读数因此一字不改）。
///
/// **照实记（超时不等于没落）**：下面那个 `ask_out` 超时也折成 [`ocall::BAD`]，而**砖可能照样
/// 到了**——`SQWARE_ROOT=fair` 那一景删掉之前量到过一格：`land` 答 `7`（期限到），紧接着
/// `operator::take` 仍取到一枚（探针读数里那一位 `got=true`）。故"`BAD` ⇒ 什么都没发生"这句
/// 不成立；要确定性就得看 `take`。那一景已删，这一格与场景无关，故留在这里。
pub fn land(
    say: PieToken,
    link: &Quay,
    host: TaskId,
    at: Where,
    name: Name,
    entry: PieToken,
    rule: Rule<Id, Id>,
    mine: bool,
    millis: Wait,
) -> Result<EntryId, u8> {
    let shipped = ocall::ship(entry, host).map_err(|_| ocall::BAD)?;
    ask_out(
        say,
        link,
        ocall::Req::Land {
            at,
            name,
            entry: shipped,
            rule,
            mine,
        },
        millis,
    )
    .map_err(|_| ocall::BAD)?
    .entry()
}

/// 客侧第二步（**分**）：在 `at` 那一块 `Pane` 里给 `name` 放一块空 `Pane`；答那一格自己的号。
pub fn part(
    say: PieToken,
    link: &Quay,
    at: Where,
    name: Name,
    millis: Wait,
) -> Result<EntryId, u8> {
    ask_out(say, link, ocall::Req::Part { at, name }, millis)
        .map_err(|_| ocall::BAD)?
        .entry()
}

/// 客侧第二步（**寻**）：把那一号背后那一枚 Pie 要过来——它经会话授进本端表，而
/// **它在本端表里的号随这条答话回来**（[`ocall::Union::Seed`]），故客人不必再扫表。
///
/// 返 `(状态, 那一格)`：状态是 [`ocall::OK`] 时第二格必有号；其余状态（查不到 / 被拒 /
/// 授不出去）第二格是 `None`——**不是"零号"**，是"这一趟没有可用的那一格"。
/// 那条路走不通（推不动 / 读不到 / 答话不是那个形状）⇒ `Err(Fail)`。
///
/// **照实记（`take` 那一手退场）**：从前它只返状态，客人随后得拿 `operator::take` 扫自己
/// 的表按"谁给的（`vestor`）"把那一枚认回来——壳里"最后那一枚"那条次序契约就是为它写的。
/// 号既然在持树者手里（`to.seed()`），就随答话过来；扫表这一手连同它为 `vestor` 撑起的
/// 那一个读者一起没了（判据没松：持树者那侧一次 `Reserve` 验"开者 = 本端 ＋ 记号 = 树路"）。
pub fn find(
    say: PieToken,
    link: &Quay,
    id: EntryId,
    millis: Wait,
) -> Result<(u8, Option<PieToken>), Fail> {
    let said = ask_out(say, link, ocall::Req::Find(id), millis)?;
    match said.code() {
        // 成功那一格必然带着那一枚（[`ocall::Union::Seed`]）：长度不是那个形状 = 读不懂 ⇒
        // 与"没走到"同一格（`Ok((码, None))` 说的只是"那一位不在"那一类）。
        ocall::OK => said
            .seed()
            .map(|seed| (ocall::OK, Some(seed)))
            .map_err(|_| Fail::Unknown),
        code => Ok((code, None)),
    }
}

/// 客侧第二步（**剪**）：把那一号剪掉。答一格状态（[`ocall::OK`] = 剪掉了）。
pub fn trim(say: PieToken, link: &Quay, id: EntryId, millis: Wait) -> Result<u8, Fail> {
    Ok(ask_out(say, link, ocall::Req::Trim(id), millis)?.code())
}

/// 客侧第二步（**列**）：看那一块 `Pane` 里有哪些**号**（[`Where::Root`] = 根那一层）。
///
/// 答话不是 [`ocall::OK`] ⇒ `Err(那一格码)`：这一条的答案体是数据，码不能当成功值带回来
/// （对照 [`find`]：那一档的码本身就是答案）。一推一读之间读不动 / 迟了 ⇒ [`ocall::BAD`]。
pub fn list(say: PieToken, link: &Quay, at: Where, millis: Wait) -> Result<Listing, u8> {
    ask_out(say, link, ocall::Req::List(at), millis)
        .map_err(|_| ocall::BAD)?
        .list()
}

/// 客侧第二步（**译**）：按一条路问"那一格是几号"——**间接寻址那一手**。
///
/// 答话是号那一形（`[0] status [1 .. 9] 号`）：答话不是 [`ocall::OK`] ⇒ `Err(那一格码)`。
/// 拿到号之后同一条路就不必再念了——其余那几条一律按号走（名字只到这一格为止）。
pub fn seek(say: PieToken, link: &Quay, road: &[Name], millis: Wait) -> Result<EntryId, u8> {
    ask_out(say, link, ocall::Req::Road(road), millis)
        .map_err(|_| ocall::BAD)?
        .entry()
}

/// 客侧第二步（**名**）：这枚号此刻叫什么。
///
/// 名字**在答话那一侧**（问话里只有号）——长短由那一帧说。
pub fn name(say: PieToken, link: &Quay, id: EntryId, millis: Wait) -> Result<Name, u8> {
    ask_out(say, link, ocall::Req::Name(id), millis)
        .map_err(|_| ocall::BAD)?
        .name()
}

// 照实记（删掉的一处：这一面的 `take`）：它从前在这儿——`find` 只答一格状态，客人于是要
// **扫自己的表**按"来源位是持树者"把刚授进来的那一枚认回来（取满足的里最后一枚）。
// `find` 的答话带上那一格之后（见本文件 `find` 的照实记）它没有读者了，按"机制退了，格也退"
// 删掉。**这是 `vestor` 在扫描里的最后一个读者**：它一走，`mail::pies()` 枚举出来的每一枚
// 就只剩 `owner ＋ mark` 两个事实有人在读，`Collect` 那一格便能收窄（乙′ 的末环）。

/// 收下树路上那一格：**答话的是谁**（[`open`] 的对偶）。
///
/// 宽度与字节序归 [`Field`](env::wire::Field) 给 [`TaskId`] 那一对 `store` / `fetch`
/// （写它的那一侧是 `programs/src/system/operator/bridge.rs` 的 `tell`）——这一格从前在五处
/// 各写一遍（那一对里记着）。
///
/// 返 `None` = 期限到了还没到 ⇒ 这条服务没接上树（客人报它自己的超时，不猜）。
pub(crate) fn hear(quay: &Quay, millis: Wait) -> Option<TaskId> {
    let link = Name::new(LINK).ok()?;
    let pier = quay.find(link)?;
    let mut buf = [0u8; TaskId::WIDTH];
    match mail::HolePie::from_token(pier.hole()).pull_timeout(&mut buf, millis) {
        Ok(n) if n == TaskId::WIDTH => TaskId::fetch(&buf),
        _ => None,
    }
}
