//! operator::client — **客侧三手**：「持树者是谁」由装配侧递一格，此后一问一答
//!
//! 三侧分家之后本文件只放**客侧三手**：「持树者是谁」由装配侧递一格，此后一问一答；两侧共用的图与说明见 [`super`] 的"载体"那一节，
//! 帧与记号见 [`crate::operator::call`]。

use env::Mark;
use env::{Name, PieToken, TaskId};
use runtime::core::port::{self, Access, Policy};
use runtime::env::mail::{self, AnyPie};

use crate::operator::Fail;
use crate::operator::call as ocall;
pub use crate::operator::{ASK_MARK, LINK, TIP_NAME};
use crate::operator::{EntryId, Listing};
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

/// 客侧第二步：问一句、取一句答（**缓冲由调用方给**，故一串号与一枚名字也走这里）。
///
/// 问话推 `say`（[`ask_hole`] 铸的那一枚，持树者读），答话从本端这条树路读（持树者写）。
/// 返**收到的帧长**；答话那一格在 `reply[0]`。
///
/// `tail` 是"随 op 变的那个号"：`land` 那一码要把**入口**捎上——它经会话交给持树者
/// （`Accord` 一份），故写进帧里的是"种在持树者表里的那个号"，那才是它认得的坐标；
/// `name` 拿它当条目的号，`list` 不看它。
pub fn ask_into(
    say: PieToken,
    link: &Quay,
    host: TaskId,
    op: u8,
    path: &[Name],
    tail: [u8; 8],
    reply: &mut [u8],
    millis: usize,
) -> Result<usize, Fail> {
    let at = Name::new(LINK).map_err(|_| Fail::Unknown)?;
    let pier = link.find(at).ok_or(Fail::Unknown)?;
    let tail = match op {
        ocall::LAND => ocall::hang(ocall::entry_in(tail), host)
            .map_err(|()| Fail::Unknown)?
            .to_bytes(),
        _ => tail,
    };
    // 孔是单槽：槽里还压着上一条时这一推会**等在门外**（`push` 满则挂），不是错误。
    mail::HolePie::from_token(say)
        .push(&ocall::pack_ask(op, path, tail))
        .map_err(|_| Fail::Unknown)?;
    pier.pull(reply, millis).map_err(|_| Fail::Unknown)
}

/// 客侧第二步（一格状态那一档）：问一句、收下那一格。返答话那一格（[`ocall::OK`] = 收下了）。
pub fn ask(
    say: PieToken,
    link: &Quay,
    host: TaskId,
    op: u8,
    path: &[Name],
    entry: PieToken,
    millis: usize,
) -> Result<u8, Fail> {
    let mut reply = [0u8; 1];
    let _ = ask_into(
        say,
        link,
        host,
        op,
        path,
        entry.to_bytes(),
        &mut reply,
        millis,
    )?;
    Ok(reply[0])
}

/// 客侧第二步（一串号那一档）：列一段路，返那一串**号**。
///
/// 答话不是 [`ocall::OK`] ⇒ `Err(那一格码)`：这一条的答案体是数据，码不能当成功值带回来
/// （对照 [`ask`]：那一档的码本身就是答案）。一推一读之间读不动 / 迟了 ⇒ [`ocall::BAD`]。
pub fn list(
    say: PieToken,
    link: &Quay,
    host: TaskId,
    path: &[Name],
    millis: usize,
) -> Result<Listing, u8> {
    let mut reply = [0u8; ocall::LIST_REPLY_LEN];
    let n = ask_into(
        say,
        link,
        host,
        ocall::LIST,
        path,
        [0u8; 8],
        &mut reply,
        millis,
    )
    .map_err(|_| ocall::BAD)?;
    ocall::read_list(&reply[..n])
}

/// 客侧第二步（**一枚号**那一档）：按一条路问"那一格是几号"——**间接寻址那一手**。
///
/// 答话是号那一形（`[0] status [1 .. 9] 号`）：答话不是 [`ocall::OK`] ⇒ `Err(那一格码)`。
/// 拿到号之后同一条路就不必再念了——其余那几条一律按号走（名字只到这一格为止）。
pub fn seek(
    say: PieToken,
    link: &Quay,
    host: TaskId,
    road: &[Name],
    millis: usize,
) -> Result<EntryId, u8> {
    let mut reply = [0u8; ocall::ID_REPLY_LEN];
    let n = ask_into(
        say,
        link,
        host,
        ocall::SEEK,
        road,
        [0u8; 8],
        &mut reply,
        millis,
    )
    .map_err(|_| ocall::BAD)?;
    ocall::read_id(&reply[..n])
}

/// 客侧第二步（一枚名字那一档）：这枚号此刻叫什么。
///
/// 名字**不走问话**（段那一格空着、尾格是条目的号），它在答话那一侧——长短由那一帧说。
pub fn name(
    say: PieToken,
    link: &Quay,
    host: TaskId,
    id: EntryId,
    millis: usize,
) -> Result<Name, u8> {
    let mut reply = [0u8; ocall::NAME_REPLY_LEN];
    let n = ask_into(
        say,
        link,
        host,
        ocall::NAME,
        &[],
        id.to_bytes(),
        &mut reply,
        millis,
    )
    .map_err(|_| ocall::BAD)?;
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

/// 收下树路上那一格：**答话的是谁**（[`tell`] 的对偶）。
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
