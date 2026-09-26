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
//!   Held(endpoint(..))              同上，但这一段关系**归本端持有**（落出作用域即放下）
//!   give(to, mark)                  只要前一半，且**把读端交出去**：本端留写端
//!   claim / find(of, mark)          只要后一半：别人交来的那一枚，按 owner ＋ mark 找回来
//! ```
//!
//! **判据两格**（`owner` ＋ `mark`），都读内核查得到的事实。编号分不开"同一位开的多枚孔"
//! ——同一个域里两枚线程各有一张表、同一个号在别人表里解析不动——记号才分得开（铸孔那一刻
//! 刻上去，随副本过线、转手不变）。这一条是原 `Quay::claim` 的正文，判据一字未改；**一律按
//! 两格问**：只按记号扫表会把"同一张表里另一枚同记号的孔"认进来。
//!
//! # 归属：两个寿命，**两个类型**——错的那个动作在 API 上不存在
//!
//! 孔的活命跟着它所在那张表（内核的规矩：`mail::release` 放下**并连派生边一起摘下**，
//! 或者那张表退场时由退场钩子沿 `sire` 摘）。而 Rust 的 `Drop` 认的是**词法作用域**——
//! 两者只在"这段关系的寿命就是这一段代码"时重合。本仓两类关系都有，故**按寿命分类型**：
//!
//! ```text
//!   Endpoint     归域：Copy、无 Drop、**没有 close**  —— 放下这个值什么也不会发生
//!   Held         有主：独占（!Copy）、Drop 放下本端那一枚
//!   裸 PieToken  归对方：give 交出去的那一枚 / claim 认下的那一枚（本端放不下，也不该放）
//! ```
//!
//! 于是两类错各自**写不出来**：
//!
//! - **提前放**（域级关系在短命作用域里被放下）：[`Endpoint`] 上根本没有"放下"这个动作、
//!   也没实现 `Drop` ⇒ 那一行写不出来；
//! - **忘了放**（有主的关系落出作用域没放）：[`Held`] 的 `Drop` 代劳 ⇒ "漏写"不是一个可能的
//!   失败模式。
//!
//! **照实记（为什么不是"给 `Endpoint` 装 `Drop`"）**：装过，撤了。本协议里两条装配路
//! （板路 / 树路）的关系是"本域活着它就得在"，而它们的值落在这些地方：`echo::register`、
//! `router::serve_board`（板、树各一条）、`uart` / `rtc` 的 `up`、名册 / 盟册那个起手闭包、
//! 持树者 `serve`——**每一处的词法结束都远早于本域退场**。装上 `Drop` 之后这些地方每一次都在
//! 关系没结束时放下，而 `release` 把派生边一起摘 ⇒ 对端表里那份（装配者转授给板 / 持树者的
//! 答话路）同时消失 ⇒ 板上那一位永远 `admit` 不上（症状是"这一位死了没人报"）——**只有跑机器
//! 才看得见**。把十二处持有者抬到"寿命等于本域"的容器里能修，可那是一张**要人一次做对的清单**，
//! 漏一处的表现照样是静默坏。故判据换成按寿命分类型：**归域的放不下，有主的忘不掉**。
//!
//! **照实记（那一刀里量过的漏，仍然收）**：路由者拒单那一趟、客户重试失败那几趟——**都不在账里**
//! （账里根本没有那一格），故从前要靠人写 `drop_lane` / `shut` 去收；它们全是**真·作用域寿命**
//! （一趟一次），故今天走 [`Held`]：`Drop` 代劳，一处也不用记。
//!
//! **类型不在号上**：这一段路是**字节**的（一枚孔不知道上面流的是什么报），故两个类型都只持号；
//! 要类型化的收发时现取一只手柄——`pair.receiver::<Rx>()` / `pair.sender::<Tx>()`。
//! 这样同一条路上换一种报不必换一个对象。
//!
//! # 这一层还装什么
//!
//! ```text
//!   lend_out                            借一枚回信孔过去、但先不推（返两格号）
//!   vested_by / opened_by / marked_as   `Reserve` 的三格：谁授的 / 谁开的 / 刻的什么
//! ```
//!
//! 它们是上面那三个动词**踩着的原语**，各有各的读者（板 / 树 / 线 / 三台服务各取所需）。
//!
//! **照实记（六具薄壳一起退场）**：这里从前还住着六具"一个调用 / 一处转发"的手——
//! `ship`（`port::ship` ＋ 一格 `VEST`）、`unship`（`mail::release`）、`push_to`
//! （`HolePie::push`）、`find`（那一趟扫表的**改名**）、`unseal_hole`（`mail::unseal_hole`
//! 并擦掉错误）、`hold`（[`Held`] 的构造）。判据是"删了不丢判断、只丢一个名字"，故
//! **一个也不留**（用户裁定）：调用点直接叫内核那几手——`mail::unseal_hole` / `mail::release` /
//! `HolePie::push` / `port::ship` / `mail::pies()` 那一趟扫 / `Held(..)` 那个字面量。
//!
//! **代价照实记**：`ship` 那一格 `R|W ＋ VEST` 从"一处"变成"每个调用点各写一遍"——少 `VEST`
//! 的症状（对端再授出那一步答 `Denied`，而两侧已经配好了对）因此在每一处随正文写了一行。
//! 判据没变，变的是它今天**写在四处外部调用点与本文件 [`endpoint`] 那一手（`seal_and_ship`）上**
//! ——就是从这具壳里分出去的五处（全仓另有板那一枚提示孔一处，它本来就直叫 `port::ship`）。
//!
//! **照实记（这几手原先住 `communication/hands.rs`）**：那个文件的前身是"注入进来那十枚
//! 函数指针的身体"（`session/hands.rs` 的 `Hands` 表，`session/call.rs` 的转发层）；注入与
//! `session` 一起退场之后，它只剩这几手，没有自己的裁决、也没有第二个读者群体 ⇒ 并回这里
//! （`communication` 从此三份：`establish` / `sender` / `receiver`）。

use core::ops::{Deref, DerefMut};

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

/// **归域的一对号**：我收的那一枚 ＋ （认到之后）我推的那一枚。
///
/// 它自己没有 `send` / `recv`——收发在 [`Sender`] / [`Receiver`] 上（`sender()` / `receiver()`
/// 现取）。它做三件事：**记着本端那一枚是几号**、**交出 seed**（"我给你的那一枚在你表里是几号"，
/// 一问一答里它随帧过去，服务端一次 `Reserve` 就验得完）、以及**延迟认领**（`claim`：先铸、
/// 先说话、后认的那一档）。
///
/// **它是 `Copy` 的、也没有 `Drop`**：这几枚号归**本域那张表**，故复制一个不改变归属，
/// 放下一个**什么也不会发生**——"提前放"这件事因此不是"要记得别做"，而是**没有可做的动作**
/// （见文件头那条照实记）。要一段**有主**的关系（作用域结束即放下）就把它收进 [`Held`]：
/// `Held(endpoint(..)?)`。
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

/// **有主的一对号**：**作用域寿命**——落出作用域就放下本端那一枚。
///
/// 与 [`Endpoint`] 的差别只有一条：**谁负责放**。这里的答案是"编译器"：`Drop` 跑
/// `mail::release`（放下本端铸的那一枚）。对端交进来的那一枚不在这里放——它归对端
/// （副本的 `sire` 指着对端表里那一枚，由对端的退场一并带走）。
///
/// **独占**（不可 `Copy`；Rust 也不允许 `Copy` 与 `Drop` 并存）⇒ "谁放"永远只有一个候选，
/// 没有"漏写一处"这种失败模式。反过来，**它不能被借出去而不放**：那正是要的。
///
/// **构造就是它的字面量**：`Held(endpoint(to, mark, claim_for)?)`——那一格"有主"由**类型**
/// 说出来，不另起一个名字（`hold` 那一手已删：它只是这一个字面量）。**真·作用域寿命的关系才
/// 用它**：客户手里的 [`Line`](crate::driver::line::client::Line)（我拿着它，那条线就归我）、
/// 线账里那一格（换主即作废），以及 `harness` 的压测台（每轮一条、每轮还回去）。板路 / 树路
/// 那一类是**域级**的，走 [`endpoint`]——见文件头那条照实记（"提前放"为什么在 API 上不存在）。
///
/// **照实记（`hold` 退场之后，构造退化成"套一层"）**：这一件原先只有一条来路——`hold`
/// （它自己先 `endpoint(..)` 铸一枚、再套上）。那一手删了之后，构造就是
/// `Held(endpoint(..)?)` 这个字面量，字段因此必须是 `pub`（不然三处调用点造不出来）。
/// 代价照实说：**同一枚号束可以套出两件 `Held`**（`Held(held.0)`，[`Endpoint`] 是 `Copy`）
/// ⇒ 两记 `release`（号还没被回收时第二记只是答错；**回收之后就是放掉别人的孔**）。
/// 这一格今天由**纪律**看着（只有 `endpoint(..)` 刚返的那一件该喂进来），不是由类型看着；
/// 要收回去，构造就得重新经过一个"铸 ＋ 持有"的动作——那就是被删掉的那具 `hold`。
/// （另：**把里面那一对号换成别的一对**（`*held = ..`）本来就写得出来——`DerefMut` 是原设计
/// 的一部分，旧的那一枚同样不会因此被放下；这一条不是这次删壳带进来的。）
///
/// 它自己不带访问器：`Deref` 到里面那一对号 ⇒ `held.rx()` / `held.tx()` / `held.claim(..)`
/// 照旧写。
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

/// **对称建立**：我铸一枚（刻 `mark`）交给 `to`，并认下 `to` 铸的、刻着同一个 `mark` 的那一枚。
///
/// `claim_for` = **等对方那一枚等多久**（不是收发期限；收发期限在 `send` / `recv` 上，每次调用
/// 各给一格）。`Wait::POLL` = 只扫一遍、不等——那一档的 `tx` 因此没有写端。
///
/// **认不到不是失败**：一段关系可以只有收的方向（单向那一档就是这么用的），故 `tx` 只是没有
/// 写端（`send` 答 `SendFail::Unbound`），而 `Err` 只留给"铸不出 / 交不出去"。
///
/// **它不"持有"什么**：返的 [`Endpoint`] 只是把本端铸的那一枚记下来（[`Copy`] 的号束）。
/// 这段关系归**本域那张表**：本域退场时一并回收——要"作用域一结束就放下"，把它收进
/// [`Held`]（`Held(endpoint(..)?)`）。
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
///
/// 它答的是我表里**这位开的、刻着那个记号的那一枚**。**多枚时给最后那一枚**——表内次序是
/// "什么时候进来的"，而交孔与推帧是两趟、且**孔先到** ⇒ 最后那一枚就是这一趟那一枚。这条
/// 次序是契约的一半，不是实现细节。
pub fn find(of: TaskId, mark: Mark) -> Option<PieToken> {
    let mut found = None;
    for p in mail::pies() {
        if p.owner == of && p.mark == mark {
            found = Some(p.token);
        }
    }
    found
}

/// **等对方那一枚**：先扫一遍、再等、醒来再扫（信标是一次事件，先扫才不会漏掉"等之前就落进来"
/// 的那一枚）。期限到 ⇒ `None`。
///
/// 它多通道那一档要的：一条服务通道一个记号，逐条认齐才算起来（`programs` 的 `ready`）。
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
/// **同一条正文今天在四处外部调用点与本函数上各写一遍**（见文件头那条照实记）。
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

// ── 剩下那一手 ──────────────────────────────────────────────────
//
// 上面那三个动词踩着的原语。**一个调用一处转发，不做裁决**——全层的规矩（谁的孔归谁、
// 认领按哪两格）在上面那几节里；这里一格裁决都没有。

/// **借一枚回信孔过去、但先不推**：返 `(本端那一枚, 对端表里那一枚)`。
///
/// **照实记（用户裁定甲′）**：`port::ship` 的 `to.seed()` 就是"**我给你的那一枚在你表里是几号**"，
/// 而从前那一版（`lend`）把它扔了 ⇒ 收方只能**扫全表**按"谁给的 ＋ 记号"把这一枚认回来。
/// 门上量出来那一扫是**每帧 ~6.5 ms**（表 16 枚 ⇒ O(n²)）。故这一手把第二格交出来，好让它
/// **随帧一起过去**；帧由调用方自己推（`mail::HolePie::from_token(..).push(..)`）。
///
/// 次序仍是契约的一半：**先铸、先交**（这一手），**再推**（下一手）。
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

// ── `Reserve` 的三格：一个调用的三个事实，一组三个等长名 ──────────────
//
// 三格是一**组**：三个名字读成同一句式的被动式事实（*这枚是谁授的 / 这扇门是谁开的 /
// 这枚被标成什么*），故**等长**（9/9/9）——原先板与树是 `probe`5 / `opened_by`9 /
// `mark_of`7，不等长本身就是"这一组还没想清楚"的信号。

reserve_reads! {
    /// **这枚是谁授的**（`Reserve` 第一格）。
    ///
    /// 转手（`Accord`）会改写这一格（root 转授过的门闩，`vestor` 会变成 root），故
    /// **不能用它认"对端是谁"**；要认"这扇门本身是谁的"，读 [`opened_by`]。
    ///
    /// **"答不出"这一格里就有"那扇门封印了"**：`Reserve` 的 `owner` 那一格带存活闸
    /// ⇒ 开者一退场，它开的门随之封印 ⇒ 这里当场答 `None`。故 `None` 只读作
    /// "这一条候选不成立"（不在我表里 / 不是孔 / 已封印），**不必再问第二个问题**。
    pub fn vested_by(entry) => vestor;
}

reserve_reads! {
    /// **这扇门是谁开的**（`Reserve` 第二格）。副本共享同一事实，转手不变。
    ///
    /// 回答"这一位客人自己交来的那一枚"就靠它；与 [`marked_as`] 合起来才分得开
    /// "同一位开的多枚孔"（那一格答"这是哪条路上的"）。
    ///
    /// 问不到那两格（这一枚**不是孔**、或它已不在表里）⇒ `None`：这一条候选不成立。
    pub fn opened_by(hole) => owner;
}

reserve_reads! {
    /// **这枚被标成什么记号**（`Reserve` 第三格）。铸者刻在孔上，副本共享、转手不变。
    ///
    /// **为什么另开一手、而不是折进 [`opened_by`] 那一格**：`opened_by` 在 `owner == 0`
    /// （引导期那批设备门闩）时把整条候选判成"不成立"、连记号一起丢；而"这一枚是不是
    /// `entry`"在 owner 0 的那批门闩上照样要答得出。
    pub fn marked_as(hole) => mark;
}
