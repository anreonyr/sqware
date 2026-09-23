//! operator::client — **客侧**：「持树者是谁」由装配侧递一格，此后一问一答
//!
//! 三侧分家之后本文件只放**客侧**：「持树者是谁」由装配侧递一格，此后一问一答；两侧共用的图
//! 与说明见 [`super`] 的"载体"那一节，帧与记号见 [`crate::operator::call`]。
//!
//! **一手对一条原语**（`land` / `part` / `find` / `trim` / `list` / `seek` / `name`）：线上与模型
//! 是同一件事的两层，客侧这一层也不再拿一个 `op` 码当参数——问什么形状由函数名说。

use env::Mark;
use env::{Name, PieToken, TaskId};
use runtime::core::port::{self, Access, Policy};
use runtime::env::mail::{self, AnyPie};

use crate::operator::Fail;
use crate::operator::call as ocall;
use crate::operator::call::Rule;
pub use crate::operator::{ASK_MARK, LINK, TIP_NAME};
use crate::operator::{EntryId, Listing, Where};
use crate::session::Quay;

/// 客侧第一步：装上树那条路（**记号就是这条路的名字**），认下对端那一枚，并收下
/// "**答话的是谁**"（[`hear`] 那一格）。
///
/// 返本端这座码头（**答话**从它走，问话走 [`ask_hole`]）与**持树者的号**。
///
/// `holder` = 客人认的对端 = **它的生我者**（孔交给它，它再转授给持树者）——注意它不是
/// 持树者：客人交出来的孔都落在生我者表里，故"持树者是谁"得由装配者告诉（见文件头）。
pub fn open(holder: TaskId, millis: usize) -> Result<(Quay, TaskId), Fail> {
    let link = Name::new(LINK).map_err(|_| Fail::Unknown)?;
    let mut quay = Quay::open(holder);
    quay.seat(link).map_err(ocall::map_seat)?;
    quay.claim(holder, Mark::of(link.as_str()), millis)
        .map_err(ocall::map_claim)?;
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
    let ask = mail::unseal_hole(ASK_MARK).map_err(|_| Fail::Unknown)?;
    let hole = mail::HolePie::from_token(ask);
    port::ship(&hole, host, Access::FETCH | Access::STORE, Policy::NONE)
        .map_err(|_| Fail::Unknown)?;
    hole.narrow(env::Permission::STORE)
        .map_err(|_| Fail::Unknown)?;
    Ok(ask)
}

/// 客侧第二步（内里那一手）：**编好的一问推上去，收一句答**。
///
/// 问话推 `say`（[`ask_hole`] 铸的那一枚，持树者读），答话从本端这条树路读（持树者写）。
/// 返**收到的帧长**；答话那一格在 `reply[0]`。缓冲由调用方给，故四种答形都走这里。
fn ask_out(
    say: PieToken,
    link: &Quay,
    ask: ocall::Ask<'_>,
    reply: &mut [u8],
    millis: usize,
) -> Result<usize, Fail> {
    let at = Name::new(LINK).map_err(|_| Fail::Unknown)?;
    let pier = link.find(at).ok_or(Fail::Unknown)?;
    let (frame, len) = ocall::pack_ask(ask);
    // 孔是单槽：槽里还压着上一条时这一推会**等在门外**（`push` 满则挂），不是错误。
    mail::HolePie::from_token(say)
        .push(&frame[..len])
        .map_err(|_| Fail::Unknown)?;
    pier.pull(reply, millis).map_err(|_| Fail::Unknown)
}

/// 客侧第二步（**落**）：在 `at` 那一块 `Pane` 里给 `name` 贴一枚 `Tile`；答**那一格自己的号**。
///
/// `entry` 是客人手里那一枚：它**经会话交给持树者**（`Accord` 一份）后才进帧——报文里走的
/// 是"种在持树者表里的那个号"，那才是它认得的坐标（见文件头与 [`ocall::ship`]）。
pub fn land(
    say: PieToken,
    link: &Quay,
    host: TaskId,
    at: Where,
    name: Name,
    entry: PieToken,
    rule: Rule,
    millis: usize,
) -> Result<EntryId, u8> {
    let shipped = ocall::ship(entry, host).map_err(|_| ocall::BAD)?;
    let mut reply = [0u8; ocall::ID_REPLY_LEN];
    let n = ask_out(
        say,
        link,
        ocall::Ask::Land {
            at,
            name,
            entry: shipped,
            rule,
        },
        &mut reply,
        millis,
    )
    .map_err(|_| ocall::BAD)?;
    ocall::read_id(&reply[..n])
}

/// 客侧第二步（**分**）：在 `at` 那一块 `Pane` 里给 `name` 放一块空 `Pane`；答那一格自己的号。
pub fn part(
    say: PieToken,
    link: &Quay,
    at: Where,
    name: Name,
    millis: usize,
) -> Result<EntryId, u8> {
    let mut reply = [0u8; ocall::ID_REPLY_LEN];
    let n = ask_out(say, link, ocall::Ask::Part { at, name }, &mut reply, millis)
        .map_err(|_| ocall::BAD)?;
    ocall::read_id(&reply[..n])
}

/// 客侧第二步（**寻**）：把那一号背后那一枚 Pie 要过来（它经会话授进本端表，见 [`take`]）。
///
/// 答的是一格状态（[`ocall::OK`] = 授出来了）——那条路走不通（推不动 / 读不到）⇒ `Err(Fail)`。
pub fn find(say: PieToken, link: &Quay, id: EntryId, millis: usize) -> Result<u8, Fail> {
    let mut reply = [0u8; 1];
    ask_out(say, link, ocall::Ask::Find(id), &mut reply, millis)?;
    Ok(reply[0])
}

/// 客侧第二步（**剪**）：把那一号剪掉。答一格状态（[`ocall::OK`] = 剪掉了）。
pub fn trim(say: PieToken, link: &Quay, id: EntryId, millis: usize) -> Result<u8, Fail> {
    let mut reply = [0u8; 1];
    ask_out(say, link, ocall::Ask::Trim(id), &mut reply, millis)?;
    Ok(reply[0])
}

/// 客侧第二步（**列**）：看那一块 `Pane` 里有哪些**号**（[`Where::Root`] = 根那一层）。
///
/// 答话不是 [`ocall::OK`] ⇒ `Err(那一格码)`：这一条的答案体是数据，码不能当成功值带回来
/// （对照 [`find`]：那一档的码本身就是答案）。一推一读之间读不动 / 迟了 ⇒ [`ocall::BAD`]。
pub fn list(say: PieToken, link: &Quay, at: Where, millis: usize) -> Result<Listing, u8> {
    let mut reply = [0u8; ocall::LIST_REPLY_LEN];
    let n = ask_out(say, link, ocall::Ask::List(at), &mut reply, millis).map_err(|_| ocall::BAD)?;
    ocall::read_list(&reply[..n])
}

/// 客侧第二步（**译**）：按一条路问"那一格是几号"——**间接寻址那一手**。
///
/// 答话是号那一形（`[0] status [1 .. 9] 号`）：答话不是 [`ocall::OK`] ⇒ `Err(那一格码)`。
/// 拿到号之后同一条路就不必再念了——其余那几条一律按号走（名字只到这一格为止）。
pub fn seek(say: PieToken, link: &Quay, road: &[Name], millis: usize) -> Result<EntryId, u8> {
    let mut reply = [0u8; ocall::ID_REPLY_LEN];
    let n =
        ask_out(say, link, ocall::Ask::Road(road), &mut reply, millis).map_err(|_| ocall::BAD)?;
    ocall::read_id(&reply[..n])
}

/// 客侧第二步（**名**）：这枚号此刻叫什么。
///
/// 名字**在答话那一侧**（问话里只有号）——长短由那一帧说。
pub fn name(say: PieToken, link: &Quay, id: EntryId, millis: usize) -> Result<Name, u8> {
    let mut reply = [0u8; ocall::NAME_REPLY_LEN];
    let n = ask_out(say, link, ocall::Ask::Name(id), &mut reply, millis).map_err(|_| ocall::BAD)?;
    ocall::read_name(&reply[..n])
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

/// 收下树路上那一格：**答话的是谁**（[`open`] 的对偶）。
///
/// 返 `None` = 期限到了还没到 ⇒ 这条服务没接上树（客人报它自己的超时，不猜）。
pub(crate) fn hear(quay: &Quay, millis: usize) -> Option<TaskId> {
    let link = Name::new(LINK).ok()?;
    let pier = quay.find(link)?;
    let mut buf = [0u8; 8];
    match mail::HolePie::from_token(pier.hole()).pull_timeout(&mut buf, millis) {
        Ok(8) => Some(TaskId::new(u64::from_le_bytes(buf) as usize)),
        _ => None,
    }
}
