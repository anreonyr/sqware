//! operator::client — **客侧三手**：「持树者是谁」由装配侧递一格，此后一问一答
//!
//! 三侧分家之后本文件只放**客侧三手**：「持树者是谁」由装配侧递一格，此后一问一答；两侧共用的图与说明见 [`super`] 的"载体"那一节，
//! 帧与记号见 [`crate::operator::call`]。

use env::{Name, PieToken, TaskId};
use runtime::core::port::{self, Access, Policy};
use runtime::env::mail::{self, AnyPie};

use crate::operator::Fail;
use crate::operator::call as ocall;
pub use crate::operator::{ASK_MARK, LINK, TIP_NAME};
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
    quay.claim(holder, link, millis).map_err(ocall::map_claim)?;
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

/// 客侧第二步：问一句、取一句答。返答话那一格（[`ocall::OK`] = 持树者收下了）。
///
/// 问话推 `say`（[`ask_hole`] 铸的那一枚，持树者读），答话从本端这条树路读（持树者写）。
///
/// `land` 那一码要把**入口**捎上：它经会话交给持树者（`Accord` 一份），故写进帧里的是
/// "种在持树者表里的那个号"——那个号才是它认得的坐标。
pub fn ask(
    say: PieToken,
    link: &Quay,
    host: TaskId,
    op: u8,
    path: &[Name],
    entry: PieToken,
    millis: usize,
) -> Result<u8, Fail> {
    let at = Name::new(LINK).map_err(|_| Fail::Unknown)?;
    let pier = link.find(at).ok_or(Fail::Unknown)?;
    let seed = match op {
        ocall::LAND => Some(ocall::hang(entry, host).map_err(|()| Fail::Unknown)?),
        _ => None,
    };
    // 孔是单槽：槽里还压着上一条时这一推会**等在门外**（`push` 满则挂），不是错误。
    mail::HolePie::from_token(say)
        .push(&ocall::pack_ask(op, path, seed))
        .map_err(|_| Fail::Unknown)?;
    let mut reply = [0u8; 1];
    match pier.pull(&mut reply, millis) {
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
