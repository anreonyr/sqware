//! 那两枚孔从"谁也没有"到"两头都在各自手里"。
//! # 为什么是"一手"
//! Mail 是单向的 ⇒ 一段关系的两半是同一件事的两面：**我铸一枚**（我读、对方往它推）＋
//! **认下对方铸的那一枚**（我写、往它推）。两件都对着同一个对端、同一条路，故是**一个动作**，
//! 不是一个流程。三种用法差别只在**要哪一半**：
//! **判据两格**（`owner` ＋ `mark`），都读内核查得到的事实。编号分不开"同一位开的多枚孔"
//! ——同一个域里两枚线程各有一张表、同一个号在别人表里解析不动——记号才分得开（铸孔那一刻
//! 刻上去，随副本过线、转手不变）。这一条是原 Quay::claim 的正文，判据一字未改；**一律按
//! 两格问**：只按记号扫表会把"同一张表里另一枚同记号的孔"认进来。
//! # 归属：两个寿命，**两个类型**——错的那个动作在 API 上不存在
//! 孔的活命跟着它所在那张表（内核的规矩：mail::release 放下**并连派生边一起摘下**，

use core::ops::{Deref, DerefMut};

use env::{Mark, Permission, PieToken, TaskId, Wait};
use runtime::core::res::port::{self, Access, Policy};
use runtime::env::mail::{self, AnyPie};

use super::super::hand::{Receiver, Sender};
use super::super::{deadline, remain};
use crate::wire::message::Message;

/// 两枚孔**还没要齐**：坏在哪一步，两格分得开
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EstablishFail {
    /// 铸不出孔（资源）
    NoHole,
    /// 这枚交不出去（没资格交 / 子集越界 / 对端已不在）
    NoSeed,
}

/// **归域的一对号**：我收的那一枚 ＋ （认到之后）我推的那一枚
/// 它自己没有 `send` / `recv`——收发在 Sender / Receiver 上（`sender()` / `receiver()`
/// 现取）。它做三件事：**记着本端那一枚是几号**、**交出 seed**（"我给你的那一枚在你表里是几号"
/// 一问一答里它随帧过去，服务端一次 `Reserve` 就验得完）、以及**延迟认领**（`claim`：先铸、
/// 先说话、后认的那一档）
/// 放下一个**什么也不会发生**——"提前放"这件事因此不是"要记得别做"，而是**没有可做的动作**
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Endpoint {
    rx: PieToken,
    tx: Option<PieToken>,
    seed: PieToken,
}

impl Endpoint {
    /// 我收的那一枚（本端铸的、交给对端的那一枚）
    pub fn rx(&self) -> PieToken {
        self.rx
    }

    /// 我推的那一枚（对端铸的、交给我的那一枚）；**还没认到 = `None`**
    pub fn tx(&self) -> Option<PieToken> {
        self.tx
    }

    /// **我交给对方的那一枚，在对方表里是几号**（port::ship 的 `to.seed()`）
    /// 一声不响地扔掉它是老毛病：收的人于是只能**扫自己的整张表**按"谁给的 ＋ 记号"把这一枚
    /// 认回来（每趟一遍全表）。随帧带过去之后，那边一次 `Reserve` 就验完——判据一字没改
    pub fn seed(&self) -> PieToken {
        self.seed
    }

    /// 类型化的收端（读面）
    pub fn receiver<M: Message>(&self) -> Receiver<M> {
        Receiver::from_token(self.rx)
    }

    /// 类型化的发端（写面）；**还没认到 = `None`**
    pub fn sender<M: Message>(&self) -> Option<Sender<M>> {
        self.tx.map(Sender::from_token)
    }

    /// **延迟认领**：先把这一枚铸出去、说上话，之后再认对端那一枚（`null` 那一档的次序是契约）
    /// 返 `true` = 到手了。**认不到不是错误**（与 endpoint 同一条口径）
    pub fn claim(&mut self, of: TaskId, mark: Mark, wait: Wait) -> bool {
        match claim(of, mark, wait) {
            Some(token) => {
                self.tx = Some(token);
                true
            }
            None => false,
        }
    }
}

/// **有主的一对号**：**作用域寿命**——落出作用域就放下本端那一枚
pub struct Held(pub Endpoint);

impl Deref for Held {
    type Target = Endpoint;

    fn deref(&self) -> &Endpoint {
        &self.0
    }
}

impl DerefMut for Held {
    fn deref_mut(&mut self) -> &mut Endpoint {
        &mut self.0
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        // **只放本端铸的那一枚**（对端那一枚归对端，见文件头）。
        let _ = mail::release(self.0.rx);
    }
}

/// **对称建立**：我铸一枚（刻 `mark`）交给 `to`，并认下 `to` 铸的、刻着同一个 `mark` 的那一枚
/// `claim_for` = **等对方那一枚等多久**（不是收发期限；收发期限在 `send` / `recv` 上，每次调用
/// 各给一格）。Wait::POLL = 只扫一遍、不等——那一档的 `tx` 因此没有写端
/// **认不到不是失败**：一段关系可以只有收的方向（单向那一档就是这么用的），故 `tx` 只是没有
/// 写端（`send` 答 SendFail::Unbound），而 `Err` 只留给"铸不出 / 交不出去"
/// **它不"持有"什么**：返的 Endpoint 只是把本端铸的那一枚记下来（Copy 的号束）
pub fn endpoint(to: TaskId, mark: Mark, claim_for: Wait) -> Result<Endpoint, EstablishFail> {
    let (rx, seed) = seal_and_ship(to, mark)?;
    Ok(Endpoint {
        rx,
        tx: claim(to, mark, claim_for),
        seed,
    })
}

/// Answer an already observed live endpoint; do not rescan for a replacement.
pub fn accept(entry: PieToken) -> Result<Endpoint, EstablishFail> {
    let (_, owner, mark) = mail::reserve(entry).map_err(|_| EstablishFail::NoSeed)?;
    let (rx, seed) = seal_and_ship(owner, mark)?;
    Ok(Endpoint {
        rx,
        tx: Some(entry),
        seed,
    })
}

/// **单向赠予**：我铸一枚（刻 `mark`），把**读端**交给 `to`，本端 `narrow(STORE)` 留写端
/// 与 endpoint 正好相反：那边交出去的是"我读的那一枚"（对端写），这边交出去的是
/// "对端读的那一枚"（本端写）。用家是"我有一位常驻的收信人、我只有话要说"那一档
/// （板 / 树各一处问话孔）
/// **`narrow` 那一手不能省**：一条路上只有一个读者——不窄下来，本端与对端都能读同一枚孔
/// 而孔是单手，谁先读谁吃掉
pub fn give(to: TaskId, mark: Mark) -> Result<PieToken, EstablishFail> {
    let hole = mail::unseal_hole(mark).map_err(|_| EstablishFail::NoHole)?;
    let pie = mail::HolePie::from_token(hole);
    port::ship(&pie, to, Access::FETCH | Access::STORE, Policy::NONE)
        .map_err(|_| EstablishFail::NoSeed)?;
    pie.narrow(Permission::STORE)
        .map_err(|_| EstablishFail::NoSeed)?;
    Ok(hole)
}

/// **扫表认领**：按 `owner` ＋ `mark` 两格找回别人交来的那一枚，**只扫一遍、不等**
/// 它答的是我表里**这位开的、刻着那个记号的那一枚**。**多枚时给最后那一枚**——表内次序是
/// 次序是契约的一半，不是实现细节
pub fn find(of: TaskId, mark: Mark) -> Option<PieToken> {
    let mut found = None;
    for p in mail::pies() {
        if p.owner == of && p.mark == mark {
            found = Some(p.token);
        }
    }
    found
}

pub fn claim(of: TaskId, mark: Mark, wait: Wait) -> Option<PieToken> {
    let until = deadline(wait);
    loop {
        if let Some(token) = find(of, mark) {
            return Some(token);
        }
        let remain = remain(until);
        if remain == Wait::POLL {
            return None;
        }
        let _ = runtime::env::unit::fall(remain);
    }
}

/// 铸一枚（刻 `mark`）交给 `to`。返 `(本端那一枚, 它在对方表里的号)`
/// 权限给满（`R|W`）**加一格 `VEST`**：对端因此可以再授出。那一格不是客气——内核那道闸是
/// "持 `VEST` 才交得出去"，而"对端把这一枚转给第三方"是**必然**发生的一步（子方只认得它的
/// 生我者，故它交出来的孔先落在生我者表里，再由生我者转授——板那条路就是这么接上的）
/// 不给 `VEST` 的症状是**转授那一步答 `Denied`**，而两侧已经配好了对，看上去像"对面坏了"
fn seal_and_ship(to: TaskId, mark: Mark) -> Result<(PieToken, PieToken), EstablishFail> {
    let hole = mail::unseal_hole(mark).map_err(|_| EstablishFail::NoHole)?;
    let pie = mail::HolePie::from_token(hole);
    match port::ship(&pie, to, Access::FETCH | Access::STORE, Policy::VEST) {
        Ok(at) => Ok((hole, at.seed())),
        Err(_) => {
            // **交不出去就当场放回来**：不留一枚没人认得的孔在本端表里。
            let _ = mail::release(hole);
            Err(EstablishFail::NoSeed)
        }
    }
}

// 上面那三个动词踩着的原语。**一个调用一处转发，不做裁决**——全层的规矩（谁的孔归谁、

/// **借一枚回信孔过去、但先不推**：返 `(本端那一枚, 对端表里那一枚)`
pub fn lend_out(entry: PieToken, mark: Mark) -> Result<(PieToken, PieToken), ()> {
    let host = opened_by(entry).ok_or(())?;
    let back = mail::unseal_hole(mark).map_err(|_| ())?;
    match port::ship(
        &mail::HolePie::from_token(back),
        host,
        Access::STORE,
        Policy::NONE,
    ) {
        Ok(to) => Ok((back, to.seed())),
        Err(_) => {
            let _ = mail::release(back);
            Err(())
        }
    }
}

// 三格是一**组**：三个名字读成同一句式的被动式事实（*这枚是谁授的 / 这扇门是谁开的 /

/// **这一枚还在不在**（**与种类无关**：孔 / 页 / 铃 / 组都答得出）。
///
/// 与上面那三格的分工：那三格读的是**孔的**来历与记号（`Reserve` 对页与铃答 `Denied`，
/// 故"这一枚不是孔"与"它已不在"在那里同形）；这一格只答**存活**。
/// 树那一层"把门牌后面那一枚交出去"要的正是这一件（判据 = 还能不能交出去，与种类无关）。
///
/// **不失败**：不在表里 / 已封印 / 号是野的 —— 一律 `false`。
pub fn alive(entry: env::PieToken) -> bool {
    runtime::env::mail::alive(entry)
}

/// **这枚是谁授的**（`Reserve` 第一格）
pub fn vested_by(entry: env::PieToken) -> Option<env::TaskId> {
    runtime::env::mail::reserve(entry)
        .ok()
        .map(|(vestor, _owner, _mark)| vestor)
}

/// **这扇门是谁开的**（`Reserve` 第二格）。副本共享同一事实，转手不变
/// 回答"这一位客人自己交来的那一枚"就靠它；与 marked_as 合起来才分得开
/// "同一位开的多枚孔"（那一格答"这是哪条路上的"）
/// 问不到那两格（这一枚**不是孔**、或它已不在表里）⇒ `None`：这一条候选不成立
pub fn opened_by(hole: env::PieToken) -> Option<env::TaskId> {
    match runtime::env::mail::reserve(hole) {
        Ok((_vestor, owner, _mark)) if owner.get() != 0 => Some(owner),
        _ => None,
    }
}

/// **这枚被标成什么记号**（`Reserve` 第三格）。铸者刻在孔上，副本共享、转手不变
/// **为什么另开一手、而不是折进 opened_by 那一格**：`opened_by` 在 `owner == 0`
/// （引导期那批设备门闩）时把整条候选判成"不成立"、连记号一起丢；而"这一枚是不是
/// `entry`"在 owner 0 的那批门闩上照样要答得出
pub fn marked_as(hole: env::PieToken) -> Option<env::Mark> {
    match runtime::env::mail::reserve(hole) {
        Ok((_vestor, _owner, mark)) => Some(mark),
        _ => None,
    }
}
