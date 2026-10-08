//! 单向通信能力的配对、显式导入和必要的唯一发现。
//!
//! 发现比较内核返回的 owner/mark 并只接受存活候选；缺失可以等待，歧义立即失败。
//! 明确交付的 token 不重新扫描，已有 tx 只核验原绑定，不自动选替代能力。
//! Mark 是每份能力引用的标签，转授可以重标记；类型、来源和权限另行验证。

use core::ops::{Deref, DerefMut};

use ::resource::port::{self, Access, Policy};
use env::{Mark, Permission, PieToken, TaskId, Wait};

use super::super::hand::{Receiver, Sender};
use crate::time::{deadline, remain};
use ::resource::raw::{alive as raw_alive, inspect, pies, reserve};
use env::pie;
use wire::Message;

/// 两枚孔**还没要齐**：坏在哪一步，两格分得开
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EstablishFail {
    /// 铸不出孔（资源）
    NoHole,
    /// 这枚交不出去（没资格交 / 子集越界 / 对端已不在）
    NoSeed,
    /// More than one live resource matched the requested role.
    Ambiguous,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DiscoveryFail {
    Missing,
    Ambiguous,
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
        Receiver::from_raw(self.rx)
    }

    /// 类型化的发端（写面）；**还没认到 = `None`**
    pub fn sender<M: Message>(&self) -> Option<Sender<M>> {
        self.tx.map(Sender::from_raw)
    }

    /// **延迟认领**：先把这一枚铸出去、说上话，之后再认对端那一枚（`null` 那一档的次序是契约）
    /// 未绑定且缺失返回 false；歧义或已有绑定失效立即失败。
    pub fn claim(&mut self, of: TaskId, mark: Mark, wait: Wait) -> Result<bool, DiscoveryFail> {
        if let Some(token) = self.tx {
            return if bound_matches(token, of, mark) {
                Ok(true)
            } else {
                Err(DiscoveryFail::Missing)
            };
        }
        match claim(of, mark, wait) {
            Ok(token) if bound_matches(token, of, mark) => {
                self.tx = Some(token);
                Ok(true)
            }
            Ok(_) => Err(DiscoveryFail::Missing),
            Err(DiscoveryFail::Missing) => Ok(false),
            Err(error @ DiscoveryFail::Ambiguous) => Err(error),
        }
    }
}

fn bound_matches(token: PieToken, of: TaskId, mark: Mark) -> bool {
    raw_alive(token)
        && reserve(token)
            .is_ok_and(|(_vestor, owner, actual_mark)| owner == of && actual_mark == mark)
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
        let _ = pie::release(self.0.rx);
    }
}

/// **对称建立**：我铸一枚（刻 `mark`）交给 `to`，并认下 `to` 铸的、刻着同一个 `mark` 的那一枚
/// `claim_for` = **等对方那一枚等多久**（不是收发期限；收发期限在 `send` / `recv` 上，每次调用
/// 各给一格）。Wait::POLL = 只扫一遍、不等——那一档的 `tx` 因此没有写端
/// **认不到不是失败**：一段关系可以只有收的方向（单向那一档就是这么用的），故 `tx` 只是没有
/// 写端（`send` 答 SendFail::Unbound）；歧义、类型不符或创建／转授失败返回错误。
/// **它不"持有"什么**：返的 Endpoint 只是把本端铸的那一枚记下来（Copy 的号束）
pub fn endpoint(to: TaskId, mark: Mark, claim_for: Wait) -> Result<Endpoint, EstablishFail> {
    let (rx, seed) = seal_and_ship(to, mark)?;
    let tx = match claim(to, mark, claim_for) {
        Ok(token) if bound_matches(token, to, mark) => Some(token),
        Ok(_) => {
            let _ = pie::revoke(to, seed);
            let _ = pie::release(rx);
            return Err(EstablishFail::NoSeed);
        }
        Err(DiscoveryFail::Missing) => None,
        Err(DiscoveryFail::Ambiguous) => {
            let _ = pie::revoke(to, seed);
            let _ = pie::release(rx);
            return Err(EstablishFail::Ambiguous);
        }
    };
    Ok(Endpoint { rx, tx, seed })
}

/// Ship a fresh endpoint without searching for the peer's matching endpoint.
/// The returned endpoint has a receive side but no bound sender; the peer can
/// explicitly return the seed created by its later `accept` call.
pub fn lend(to: TaskId, mark: Mark) -> Result<Endpoint, EstablishFail> {
    let (rx, seed) = seal_and_ship(to, mark)?;
    Ok(Endpoint { rx, tx: None, seed })
}

/// Answer an already observed live endpoint; do not rescan for a replacement.
pub fn accept(entry: PieToken) -> Result<Endpoint, EstablishFail> {
    if !raw_alive(entry) {
        return Err(EstablishFail::NoSeed);
    }
    let (_, owner, mark) = reserve(entry).map_err(|_| EstablishFail::NoSeed)?;
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
    give_at(to, mark).map(|(local, _remote)| local)
}

/// Create a one-way ask hole: local STORE capability and the peer's FETCH seed.
pub fn give_at(to: TaskId, mark: Mark) -> Result<(PieToken, PieToken), EstablishFail> {
    let hole = pie::unseal_hole(mark).map_err(|_| EstablishFail::NoHole)?;
    let remote = match port::ship(hole, to, Access::FETCH | Access::STORE, Policy::NONE) {
        Ok(at) => at.seed(),
        Err(_) => {
            let _ = pie::release(hole);
            return Err(EstablishFail::NoSeed);
        }
    };
    if env::pie::narrow(hole, Permission::STORE).is_err() {
        let _ = pie::revoke(to, remote);
        let _ = pie::release(hole);
        return Err(EstablishFail::NoSeed);
    }
    Ok((hole, remote))
}

/// Find the unique live resource matching its kernel-verified owner and mark.
pub fn find(of: TaskId, mark: Mark) -> Result<PieToken, DiscoveryFail> {
    let mut found = None;
    for p in pies() {
        if !raw_alive(p.token) {
            continue;
        }
        if let Ok((_vestor, owner, actual_mark)) = inspect(p.token) {
            if owner != of || actual_mark != mark {
                continue;
            }
            if found.is_some() {
                return Err(DiscoveryFail::Ambiguous);
            }
            found = Some(p.token);
        }
    }
    found.ok_or(DiscoveryFail::Missing)
}

pub fn claim(of: TaskId, mark: Mark, wait: Wait) -> Result<PieToken, DiscoveryFail> {
    let until = deadline(wait);
    loop {
        match find(of, mark) {
            Ok(token) => return Ok(token),
            Err(DiscoveryFail::Ambiguous) => return Err(DiscoveryFail::Ambiguous),
            Err(DiscoveryFail::Missing) => {}
        }
        let remain = remain(until);
        if remain == Wait::POLL {
            return Err(DiscoveryFail::Missing);
        }
        let _ = env::unit::fall(remain);
    }
}

/// 铸一枚（刻 `mark`）交给 `to`。返 `(本端那一枚, 它在对方表里的号)`
/// 权限给满（`R|W`）**加一格 `VEST`**：对端因此可以再授出。那一格不是客气——内核那道闸是
/// "持 `VEST` 才交得出去"，而"对端把这一枚转给第三方"是**必然**发生的一步（子方只认得它的
/// 生我者，故它交出来的孔先落在生我者表里，再由生我者转授——板那条路就是这么接上的）
/// 不给 `VEST` 的症状是**转授那一步答 `Denied`**，而两侧已经配好了对，看上去像"对面坏了"
fn seal_and_ship(to: TaskId, mark: Mark) -> Result<(PieToken, PieToken), EstablishFail> {
    let hole = pie::unseal_hole(mark).map_err(|_| EstablishFail::NoHole)?;
    match port::ship(hole, to, Access::FETCH | Access::STORE, Policy::VEST) {
        Ok(at) => Ok((hole, at.seed())),
        Err(_) => {
            // **交不出去就当场放回来**：不留一枚没人认得的孔在本端表里。
            let _ = pie::release(hole);
            Err(EstablishFail::NoSeed)
        }
    }
}

// 上面那三个动词踩着的原语。**一个调用一处转发，不做裁决**——全层的规矩（谁的孔归谁、

/// **借一枚回信孔过去、但先不推**：返 `(本端那一枚, 对端表里那一枚)`
pub fn lend_out(entry: PieToken, mark: Mark) -> Result<(PieToken, PieToken), ()> {
    let host = opened_by(entry).ok_or(())?;
    let back = pie::unseal_hole(mark).map_err(|_| ())?;
    match port::ship(back, host, Access::STORE, Policy::NONE) {
        Ok(to) => Ok((back, to.seed())),
        Err(_) => {
            let _ = pie::release(back);
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
    ::resource::raw::alive(entry)
}

/// **这枚是谁授的**（`Reserve` 第一格）
pub fn vested_by(entry: env::PieToken) -> Option<env::TaskId> {
    reserve(entry).ok().map(|(vestor, _owner, _mark)| vestor)
}

/// **这扇门是谁开的**（`Reserve` 第二格）。副本共享同一事实，转手不变
/// 回答"这一位客人自己交来的那一枚"就靠它；与 marked_as 合起来才分得开
/// "同一位开的多枚孔"（那一格答"这是哪条路上的"）
/// 问不到那两格（这一枚**不是孔**、或它已不在表里）⇒ `None`：这一条候选不成立
pub fn opened_by(hole: env::PieToken) -> Option<env::TaskId> {
    match reserve(hole) {
        Ok((_vestor, owner, _mark)) if owner.get() != 0 => Some(owner),
        _ => None,
    }
}

/// 查询这份 Hole 能力引用当前携带的标记；派生引用可以使用不同标记。
/// **为什么另开一手、而不是折进 opened_by 那一格**：`opened_by` 在 `owner == 0`
/// （引导期那批设备门闩）时把整条候选判成"不成立"、连记号一起丢；而"这一枚是不是
/// `entry`"在 owner 0 的那批门闩上照样要答得出
pub fn marked_as(hole: env::PieToken) -> Option<env::Mark> {
    match reserve(hole) {
        Ok((_vestor, _owner, mark)) => Some(mark),
        _ => None,
    }
}
