//! establish — **关系怎么建立**：那两枚孔从"谁也没有"到"两头都在各自手里"。
//!
//! # 为什么是"一手"
//!
//! Mail 是单向的 ⇒ 一段关系的两半是同一件事的两面：**我铸一枚**（我读、对方往它推）＋
//! **认下对方铸的那一枚**（我写、往它推）。两件都对着同一个对端、同一条路，故是**一个动作**，
//! 不是一个流程。今天的三种形状（对称 / 单向 seat / 单向赠予）差别只在**要哪一半**：
//!
//! ```text
//!   endpoint(to, mark, claim_for)   两半都要：我铸我读 ＋ 认对方那枚（认不到就只有收的方向）
//!   give(to, mark)                  只要前一半，且**把读端交出去**：本端留写端
//!   find(of, mark)                  只要后一半：别人交来的那一枚，按 owner ＋ mark 找回来
//! ```
//!
//! **判据两格**（`owner` ＋ `mark`），都读内核查得到的事实。编号分不开"同一位开的多枚孔"
//! ——同一个域里两枚线程各有一张表、同一个号在别人表里解析不动——记号才分得开（铸孔那一刻
//! 刻上去，随副本过线、转手不变）。这一条是原 `Quay::claim` 的正文，判据一字未改。
//!
//! # 归属：为什么要有一件东西**持有**那一对
//!
//! `endpoint` 铸的那两枚**归本端**：本端铸的那一枚要放下（不放就是"每失败一次、表里多两枚，
//! 直到本域退场"——原 `Quay` 没有 `Drop`，那笔账写在 `programs` 的 `desk.rs::drop_lane` 上）。
//! 而服务端手里那些号（`guest.ask()` / `guest.reply()`）**归别人**——放下它就把客人的孔收掉。
//! 同一个 `Receiver` 两种归属 ⇒ 归属必须由**持有者**说，不能由手柄自己猜：
//! [`Pair`] 就是"本端铸的那一对"的持有者，`Drop` 时放下**本端那一枚**（对端那一枚归对端，
//! 由它的退场一并带走）；从裸号建的手柄不进 `Pair`，也不 `Drop`。

use env::{Mark, Permission, PieToken, TaskId, Wait};
use runtime::core::port::{self, Access, Policy};
use runtime::env::mail::{self, AnyPie};

use super::receiver::Receiver;
use super::sender::Sender;
use super::{deadline, remain};
use crate::message::Message;

/// 两枚孔**还没要齐**：坏在哪一步，两格分得开。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EstablishFail {
    /// 铸不出孔（资源）。
    NoHole,
    /// 这枚交不出去（没资格交 / 子集越界 / 对端已不在）。
    NoSeed,
}

/// **本端铸的那一对**：我推的那一枚 ＋ 我收的那一枚。
///
/// 它自己没有 `send` / `recv`——收发在 [`Sender`] / [`Receiver`] 上（`tx()` / `rx()` 取）。
/// 它只做两件事：**持有**（`Drop` 时放下本端那一枚）与**交出 seed**
/// （"我给你的那一枚在你表里是几号"，一问一答里它随帧过去，服务端一次 `Reserve` 就验得完）。
pub struct Pair<Tx: Message, Rx: Message> {
    tx: Sender<Tx>,
    rx: Receiver<Rx>,
    seed: PieToken,
}

impl<Tx: Message, Rx: Message> Pair<Tx, Rx> {
    /// 我推的那一枚（对端铸的、交给我的那一枚）。
    pub fn tx(&self) -> &Sender<Tx> {
        &self.tx
    }

    /// 我收的那一枚（本端铸的、交给对端的那一枚）。
    pub fn rx(&self) -> &Receiver<Rx> {
        &self.rx
    }

    /// **我交给对方的那一枚，在对方表里是几号**（`port::ship` 的 `to.seed()`）。
    ///
    /// 一声不响地扔掉它是老毛病：收的人于是只能**扫自己的整张表**按"谁给的 ＋ 记号"把这一枚
    /// 认回来（每趟一遍全表）。随帧带过去之后，那边一次 `Reserve` 就验完——判据一字没改。
    pub fn seed(&self) -> PieToken {
        self.seed
    }
}

impl<Tx: Message, Rx: Message> Drop for Pair<Tx, Rx> {
    fn drop(&mut self) {
        // **只放本端铸的那一枚**：对端交进来的那一枚归对端（它由对端的退场一并带走，
        // 副本的 `sire` 指着对端表里那一枚）。
        let _ = mail::release(self.rx.hole());
    }
}

/// **对称建立**：我铸一枚（刻 `mark`）交给 `to`，并认下 `to` 铸的、刻着同一个 `mark` 的那一枚。
///
/// `claim_for` = **等对方那一枚等多久**（不是收发期限；收发期限在 `send` / `recv` 上，
/// 每次调用各给一格）。`Wait::POLL` = 只扫一遍、不等——那一档的 `tx` 因此没有写端。
///
/// **认不到不是失败**：一段关系可以只有收的方向（单向那一档就是这么用的），故 `tx` 只是
/// 没有写端（`send` 答 `SendFail::Unbound`），而 `Err` 只留给"铸不出 / 交不出去"。
pub fn endpoint<Tx: Message, Rx: Message>(
    to: TaskId,
    mark: Mark,
    claim_for: Wait,
) -> Result<Pair<Tx, Rx>, EstablishFail> {
    let (hole, seed) = seal_and_ship(to, mark)?;
    let tx = match claim(to, mark, claim_for) {
        Some(token) => Sender::from_token(token),
        None => Sender::unbound(),
    };
    Ok(Pair {
        tx,
        rx: Receiver::from_token(hole),
        seed,
    })
}

/// **单向赠予**：我铸一枚（刻 `mark`），把**读端**交给 `to`，本端 `narrow(STORE)` 留写端。
///
/// 与 [`endpoint`] 正好相反：那边交出去的是"我读的那一枚"（对端写），这边交出去的是
/// "对端读的那一枚"（本端写）。用家是"我有一位常驻的收信人、我只有话要说"那一档
/// （板 / 树各一处问话孔）。
///
/// **`narrow` 那一手不能省**：一条路上只有一个读者——不窄下来，本端与对端都能读同一枚孔，
/// 而孔是单槽，谁先读谁吃掉（原正文事实 2）。
pub fn give<S: Message>(to: TaskId, mark: Mark) -> Result<Sender<S>, EstablishFail> {
    let hole = mail::unseal_hole(mark).map_err(|_| EstablishFail::NoHole)?;
    let pie = mail::HolePie::from_token(hole);
    port::ship(&pie, to, Access::FETCH | Access::STORE, Policy::NONE)
        .map_err(|_| EstablishFail::NoSeed)?;
    pie.narrow(Permission::STORE)
        .map_err(|_| EstablishFail::NoSeed)?;
    Ok(Sender::from_token(hole))
}

/// **扫表认领**：按 `owner` ＋ `mark` 两格找回别人交来的那一枚，**只扫一遍、不等**。
///
/// 它是一问一答那一档要的（客人借一枚回信孔过来、交完就推帧，收的人手里没有泊位可归位，
/// 只要那一枚号）；要"等到齐"的那一档走 [`endpoint`]。
pub fn find<M: Message>(of: TaskId, mark: Mark) -> Option<Sender<M>> {
    scan(of, mark).map(Sender::from_token)
}

/// 铸一枚（刻 `mark`）交给 `to`。返 `(本端那一枚, 它在对方表里的号)`。
///
/// 权限给满（`R|W`）**加一格 `VEST`**：对端因此可以再授出。那一格不是客气——内核那道闸是
/// "持 `VEST` 才交得出去"，而"对端把这一枚转给第三方"是**必然**发生的一步（子方只认得它的
/// 生我者，故它交出来的孔先落在生我者表里，再由生我者转授——板那条路就是这么接上的）。
/// 不给 `VEST` 的症状是**转授那一步答 `Denied`**，而两侧已经配好了对，看上去像"对面坏了"。
fn seal_and_ship(to: TaskId, mark: Mark) -> Result<(PieToken, PieToken), EstablishFail> {
    let hole = mail::unseal_hole(mark).map_err(|_| EstablishFail::NoHole)?;
    match port::ship(
        &mail::HolePie::from_token(hole),
        to,
        Access::FETCH | Access::STORE,
        Policy::VEST,
    ) {
        Ok(at) => Ok((hole, at.seed())),
        Err(_) => {
            // **交不出去就当场放回来**：不留一枚没人认得的孔在本端表里。
            let _ = mail::release(hole);
            Err(EstablishFail::NoSeed)
        }
    }
}

/// 认下 `of` 交给我的、刻着 `mark` 的那一枚：**先扫再等**，醒来再扫。
///
/// 那一序不能反：信标是一次事件，先扫过一遍才不会漏掉"等之前就已经落进来"的那一枚。
/// 期限到 ⇒ `None`（**不是错误**，见 [`endpoint`]）。
fn claim(of: TaskId, mark: Mark, wait: Wait) -> Option<PieToken> {
    let until = deadline(wait);
    loop {
        if let Some(token) = scan(of, mark) {
            return Some(token);
        }
        let remain = remain(until);
        if remain == Wait::POLL {
            return None;
        }
        // 有界等（`Wait::Forever` = 永久）。返回**只是提示**：真醒还是期限到，由下一轮说了算。
        let _ = runtime::env::unit::fall(remain);
    }
}

/// 我表里**这位开的、刻着那个记号的那一枚**。**多枚时给最后那一枚**——表内次序是"什么时候
/// 进来的"，而交孔与推帧是两趟、且**孔先到** ⇒ 最后那一枚就是这一趟那一枚。这条次序是契约的
/// 一半，不是实现细节。
///
/// **一律按两格问**：只按记号扫表会把"同一张表里另一枚同记号的孔"认进来——那个洞由编译期
/// 断言（"两枚记号必须不同"）兜着，而"靠记号全局不撞"是隐性规矩，不该留在代码里。
fn scan(of: TaskId, mark: Mark) -> Option<PieToken> {
    let mut found = None;
    for p in mail::pies() {
        if p.owner == of && p.mark == mark {
            found = Some(p.token);
        }
    }
    found
}
