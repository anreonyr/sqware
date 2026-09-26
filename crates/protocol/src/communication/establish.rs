//! establish — **关系怎么建立**：那两枚孔从"谁也没有"到"两头都在各自手里"。
//!
//! # 为什么是"一手"
//!
//! Mail 是单向的 ⇒ 一段关系的两半是同一件事的两面：**我铸一枚**（我读、对方往它推）＋
//! **认下对方铸的那一枚**（我写、往它推）。两件都对着同一个对端、同一条路，故是**一个动作**，
//! 不是一个流程。三种用法差别只在**要哪一半**：
//!
//! ```text
//!   endpoint(to, mark, claim_for)   两半都要：我铸我读 ＋ 认对方那枚（认不到就只有收的方向）
//!   give(to, mark)                  只要前一半，且**把读端交出去**：本端留写端
//!   claim / find(of, mark)          只要后一半：别人交来的那一枚，按 owner ＋ mark 找回来
//! ```
//!
//! **判据两格**（`owner` ＋ `mark`），都读内核查得到的事实。编号分不开"同一位开的多枚孔"
//! ——同一个域里两枚线程各有一张表、同一个号在别人表里解析不动——记号才分得开（铸孔那一刻
//! 刻上去，随副本过线、转手不变）。这一条是原 `Quay::claim` 的正文，判据一字未改；**一律按
//! 两格问**：只按记号扫表会把"同一张表里另一枚同记号的孔"认进来。
//!
//! # 归属：那一对孔归**本域那张表**，不归词法作用域
//!
//! `endpoint` 铸的这一枚**归本端**。可"本端"是**哪一枚线程**，不是"哪一段代码"：孔的活命
//! 跟着它所在那张表（内核的规矩），故**放下它是本域自己的事、要明说**（[`Endpoint::close`]），
//! 不明说就活到本域退场。
//!
//! **照实记（`Drop` 装了又撤）**：这一版先给它装了 `impl Drop`（"持有者落出作用域就放下本端
//! 那一枚"），写完整个仓才看清它**系统性放早了**——本协议里两条装配路（板路 / 树路）的关系是
//! "本域活着它就得在"，而它们的 `Endpoint` 落在这些地方：`echo::register`、`router::serve_board`
//! （板、树各一条）、`uart` / `rtc` 的 `up`、名册 / 盟册那个起手闭包、持树者 `serve`——**每一处
//! 的词法结束都远早于本域退场**。而 `mail::release` 会**连派生边一起摘下**（`gate::cull`）：
//! 本端一放，对端表里那份（装配者转授给板 / 持树者的答话路）**同时消失** ⇒ 板上那一位永远
//! `admit` 不上（症状是"这一位死了没人报"），客人自己那半条路也当场读不动。
//! 故这一手**不能由作用域替人做**：`close` 是被叫的，`Drop` 是被猜的。
//!
//! 而服务端手里那些号（`guest.ask()` / `guest.reply()`）**归别人**——放下它就把客人的孔收掉。
//! 两种归属都靠**读的人**说清：从裸号建的手柄（`Sender::from_token` / `Receiver::from_token`）
//! 不带 `close`。
//!
//! **类型不在 `Endpoint` 上**：这一段路是**字节**的（一枚孔不知道上面流的是什么报），故
//! `Endpoint` 只持号；要类型化的收发时现取一只手柄——`pair.receiver::<Rx>()` /
//! `pair.sender::<Tx>()`。这样同一条路上换一种报不必换一个对象。

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

/// **本端铸的那一对**：我收的那一枚 ＋ （认到之后）我推的那一枚。
///
/// 它自己没有 `send` / `recv`——收发在 [`Sender`] / [`Receiver`] 上（`sender()` / `receiver()`
/// 现取）。它只做三件事：**记着本端那一枚**（[`Endpoint::close`] 放的就是它）、**交出 seed**
/// （"我给你的那一枚在你表里是几号"，一问一答里它随帧过去，服务端一次 `Reserve` 就验得完）、
/// 以及**延迟认领**（`claim`：先铸、先说话、后认的那一档）。
///
/// **是 `Copy` 的**：它只是几枚号，复制一个不改变归属（孔归本域那张表）——"放下"永远只有
/// [`Endpoint::close`] 一个入口，不会因为多了一句 `let` 而多放一次。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Endpoint {
    rx: PieToken,
    tx: Option<PieToken>,
    seed: PieToken,
}

impl Endpoint {
    /// 我收的那一枚（本端铸的、交给对端的那一枚）。
    pub fn rx(&self) -> PieToken {
        self.rx
    }

    /// 我推的那一枚（对端铸的、交给我的那一枚）；**还没认到 = `None`**。
    pub fn tx(&self) -> Option<PieToken> {
        self.tx
    }

    /// **我交给对方的那一枚，在对方表里是几号**（`port::ship` 的 `to.seed()`）。
    ///
    /// 一声不响地扔掉它是老毛病：收的人于是只能**扫自己的整张表**按"谁给的 ＋ 记号"把这一枚
    /// 认回来（每趟一遍全表）。随帧带过去之后，那边一次 `Reserve` 就验完——判据一字没改。
    pub fn seed(&self) -> PieToken {
        self.seed
    }

    /// 类型化的收端（读面）。
    pub fn receiver<M: Message>(&self) -> Receiver<M> {
        Receiver::from_token(self.rx)
    }

    /// 类型化的发端（写面）；**还没认到 = `None`**。
    pub fn sender<M: Message>(&self) -> Option<Sender<M>> {
        self.tx.map(Sender::from_token)
    }

    /// **放下本端那一枚**（这段关系到此为止）。
    ///
    /// **只有这一条出口**：不放就活到本域退场（孔的活命跟着它所在那张表，见文件头那条照实记）
    /// ——这是**故意**的：板路 / 树路那两条关系要活到本域收场，而它们的 `Endpoint` 落在好几个
    /// 短命作用域里（`up` / `register` / 起手闭包），**交给 `Drop` 就等于交给词法猜**。
    /// 对端交进来的那一枚不在这里放：它归对端（副本的 `sire` 指着对端表里那一枚，
    /// 由对端的退场一并带走）。
    pub fn close(self) {
        let _ = mail::release(self.rx);
    }

    /// **延迟认领**：先把这一枚铸出去、说上话，之后再认对端那一枚（`null` 那一档的次序是契约）。
    ///
    /// 返 `true` = 到手了。**认不到不是错误**（与 [`endpoint`] 同一条口径）。
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

/// **对称建立**：我铸一枚（刻 `mark`）交给 `to`，并认下 `to` 铸的、刻着同一个 `mark` 的那一枚。
///
/// `claim_for` = **等对方那一枚等多久**（不是收发期限；收发期限在 `send` / `recv` 上，每次调用
/// 各给一格）。`Wait::POLL` = 只扫一遍、不等——那一档的 `tx` 因此没有写端。
///
/// **认不到不是失败**：一段关系可以只有收的方向（单向那一档就是这么用的），故 `tx` 只是没有
/// 写端（`send` 答 `SendFail::Unbound`），而 `Err` 只留给"铸不出 / 交不出去"。
///
/// **它不"持有"什么**：返的 [`Endpoint`] 只是把本端铸的那一枚记下来，好让本域将来能
/// [`Endpoint::close`]——不 `close` 就活到本域退场（见文件头那条照实记）。
pub fn endpoint(to: TaskId, mark: Mark, claim_for: Wait) -> Result<Endpoint, EstablishFail> {
    let (rx, seed) = seal_and_ship(to, mark)?;
    Ok(Endpoint {
        rx,
        tx: claim(to, mark, claim_for),
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
/// 而孔是单槽，谁先读谁吃掉。
pub fn give(to: TaskId, mark: Mark) -> Result<PieToken, EstablishFail> {
    let hole = mail::unseal_hole(mark).map_err(|_| EstablishFail::NoHole)?;
    let pie = mail::HolePie::from_token(hole);
    port::ship(&pie, to, Access::FETCH | Access::STORE, Policy::NONE)
        .map_err(|_| EstablishFail::NoSeed)?;
    pie.narrow(Permission::STORE)
        .map_err(|_| EstablishFail::NoSeed)?;
    Ok(hole)
}

/// **扫表认领**：按 `owner` ＋ `mark` 两格找回别人交来的那一枚，**只扫一遍、不等**。
pub fn find(of: TaskId, mark: Mark) -> Option<PieToken> {
    scan(of, mark)
}

/// **等对方那一枚**：先扫一遍、再等、醒来再扫（信标是一次事件，先扫才不会漏掉"等之前就落进来"
/// 的那一枚）。期限到 ⇒ `None`。
///
/// 它多通道那一档要的：一条服务通道一个记号，逐条认齐才算起来（`programs` 的 `ready`）。
pub fn claim(of: TaskId, mark: Mark, wait: Wait) -> Option<PieToken> {
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

/// 我表里**这位开的、刻着那个记号的那一枚**。**多枚时给最后那一枚**——表内次序是"什么时候
/// 进来的"，而交孔与推帧是两趟、且**孔先到** ⇒ 最后那一枚就是这一趟那一枚。这条次序是契约的
/// 一半，不是实现细节。
fn scan(of: TaskId, mark: Mark) -> Option<PieToken> {
    let mut found = None;
    for p in mail::pies() {
        if p.owner == of && p.mark == mark {
            found = Some(p.token);
        }
    }
    found
}
