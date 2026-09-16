//! board — **板那两半**：板侧（[`attach`] 起待客线程 + 转授），客侧（[`open`] / [`ask`] / [`take`]）。
//!
//! 板是**装配者那个域里的一份账**（[`BOARD`]，每位客人的待客线程共享它），客人是服务域里
//! 的一枚线程。两侧各用会话的一侧（[`Quay::pair`] 的两半）：
//!
//! ```text
//!   装配者（root）                            客人（服务域）              板线程
//!   quay.seat(名字) + quay.claim(me)  ─────▶  pair(名字)  ── 交一枚孔 ──▶  seat(名字)
//!     得到客人那一枚（客人读答话）              得到板那一枚（客人写问话）   claim(客人) 认出写端
//!   转授：把客人那一枚 Ship 给板线程 ───────────────────────────────────▶  写端到手
//!                                             post(Query) ──────────────▶  pull ⇒ 交给板
//!                                             pull(Reply) ◀──────────────  post(Reply)
//! ```
//!
//! **为什么中间要装配者过一手**：客人只认得它的生我者（孔是交给"生我者"的），板线程不是
//! 它的生我者 ⇒ 客人交出来的那一枚落在装配者表里。装配者把它**转授**给板线程——装配者本来
//! 就是设备门闩的第一个持有者与转授者，这里走的是同一条路。转授的是**客人开的那扇门**：
//! 副本共享 `owner`（`session` 事实 3），故板那一侧"按 `owner` 认领"照样认得出它。
//!
//! # 一问一答的次序
//!
//! ```text
//!   1  装配者在本域与客人那座码头上加一条板泊位（本端交出自己那一枚 —— 客人往它写问话）
//!   2  认领：客人交出来的那一枚落进本域表里（客人只认得生我者，故先落这里）
//!   3  起板线程，再把那一枚转授给它 —— 板从此握着"往客人写答话"的那一扇门
//! ```
//!
//! 三步都在 [`attach`] 里，**次序即契约**：第 2 步之前板线程还没起（起了也无用——它等的
//! 就是这一枚），第 3 步之前客人那一枚还没到（它由客人在 `pair` 里交出来）。
//!
//! # 字节长什么样
//!
//! 帧形、三个动作码、一处上界、答话那一格全在 [`protocol::board::call`]；本文件只做
//! "读一条 → 交给板 → 回一句"，一个字节都不自己编。

use core::mem;

use env::{Name, PieToken, TaskId};
use protocol::board::call as bcall;
use protocol::board::{Board, Fail};
use protocol::session::Quay;
use runtime::core::lock::Lock;
use runtime::core::port::{self, Access, Policy};
use runtime::core::unit::{self, Join};
use runtime::env::mail;

/// 板那条通道的名字：**两侧同一个**（泊位自己的坐标，不进报文）。
pub const LINK: &str = "board";

/// 客人等板、等答的上限（毫秒）。**必须有界**：对面死在头几步时这边不能陪着挂死。
pub const WAIT_MS: usize = 1000;

/// 板线程等一问的上限：`usize::MAX` = **真挂起**（孔有唤醒，不空转）。
///
/// 代价照实说：客人退场这件事板看不出来（那一扇门的副本还在板手里），故这枚线程不自己
/// 收摊——随本域退场一起回收。要它自己认出来，得靠"对端没了"那一格（还没有）。
const SERVE_MS: usize = usize::MAX;

/// 板上那本账：**本域唯一一份**，每位客人的待客线程共享。
///
/// 临界区 = 一次查牌 / 改牌，故用用户态自旋锁（`runtime::core::lock` 就是为"同域跨任务
/// 共享一张表"写的）。板**不住在客人那一侧**：名字是全局的，账只能有一份。
static BOARD: Lock<Board> = Lock::new(bcall::board());

// ── 板侧（装配者与本域的板线程）──────────────────────────────

/// 把板接上一位客人：**三步**（见文件头"一问一答的次序"）。
///
/// `me` = 本域（装配者）自己的号——**客人交出来的那一枚就落在它表里**（客人把孔交给
/// 生我者）；`client` = 客人（服务域的主线程 = 装配者刚产出的代表线程）。
///
/// 返 `Err(哪一步)`：三种因（名字非法 / 席位满 / 等不到客人的那一枚）对调用方是同一件事
/// ——**这条服务没接上板**——但"死在哪一步"正是装配诊断要的那一格（与 `service::step` 同款）。
pub fn attach(quay: &mut Quay, me: TaskId, client: TaskId, ms: usize) -> Result<(), &'static str> {
    let link = Name::new(LINK).map_err(|_| "board:name")?;
    // 1. 本端那一枚交出去（`peer` 是本域自己 ⇒ 这一枚落在本域表里，客人拿不到它，
    //    也不需要：客人往板那一枚写问话）。
    quay.seat(link, ms).map_err(|_| "board:seat")?;
    // 2. 认领客人交出来的那一枚。**次序**：它比 `records` 那条后到，而 `records` 的写端
    //    已经用掉了 ⇒ 这一枚认在板泊位上（一枚孔只配一条泊位）。
    quay.claim(me, ms).map_err(|_| "board:claim")?;
    // 3. 起板线程，再把那一枚**转授**给它。
    let board = serve(client);
    hand(quay, board).map_err(|()| "board:hand")
}

/// 起一枚待客线程：对端是这位客人。
///
/// 客人是谁**从外面给**（装配者刚起了它的域）；板线程自己不去猜——它认的是"我表里出现了
/// 一枚不是我开的孔"（`Quay::claim` 的判据）。板不返回（长期待客），故这里丢掉那个
/// [`Join`]：不等它的结果，它的死活随本域。
fn serve(client: TaskId) -> TaskId {
    let node: Join<()> = unit::closure(move || host(client));
    let id = node.id();
    mem::forget(node);
    id
}

/// 把**客人交出来的那一枚**转授给板线程。
///
/// 转授的是"客人开的那扇门"（`owner` 是客人），板那侧认领时认的正是它。
///
/// 子集只给 `R|W`，**不加 `VEST`**：板线程用这一枚读写答话，不需要再授出——一分不多。
/// 本域自己那一份转授之后**不收**：内核那侧"交出"（`CAGE`）要求源枚自己带 `CAGE`，而客人
/// 给过来的这一枚没有（`seat` 给的是 `R|W|VEST`）；收它要多一条 `release`，而这一步之后
/// 没有任何东西再碰它——本域常驻，随域退场一起回收。
fn hand(quay: &Quay, board: TaskId) -> Result<(), ()> {
    let link = Name::new(LINK).map_err(|_| ())?;
    let pier = quay.find(link).ok_or(())?;
    let hole = mail::HolePie::from_token(pier.at_peer().get());
    port::ship(&hole, board, Access::READ | Access::WRITE, Policy::NONE)
        .map(|_| ())
        .map_err(|_| ())
}

/// 待客：装自己那一枚（**板读**），认出客人交上来的那一枚（**板写**），然后一问一答。
fn host(client: TaskId) {
    let Ok(link) = Name::new(LINK) else {
        return;
    };
    let mut quay = Quay::open(client);
    // 交出本端那一枚：**客人往它写问话**。`seat` 不写字条，故那一枚的槽是空的——
    // 客人的第一推直接进得去（写字条的那条路已经拆掉，见 `session` 事实 4）。
    if quay.seat(link, WAIT_MS).is_err() {
        return;
    }
    // 认出客人交上来的那一枚：它由装配者转授过来，`owner` **仍是客人** ⇒ 判据认得出。
    if quay.claim(client, WAIT_MS).is_err() {
        return;
    }
    let Some(pier) = quay.find_pier(link) else {
        return;
    };
    loop {
        let mut buf = [0u8; bcall::ASK];
        let Ok(n) = pier.pull(&mut buf, SERVE_MS) else {
            return;
        };
        let Some(want) = buf.get(..n) else {
            return;
        };
        let answer = BOARD.with(|board| answer(board, want, pier.peer()));
        if pier.post(&answer).is_err() {
            return;
        }
    }
}

/// 把一条问交给板，编出一句答（**一格**：读不懂也答，答 `BAD`）。
fn answer(board: &mut Board, want: &[u8], who: TaskId) -> [u8; 1] {
    let Some((op, name, seed)) = bcall::unpack(want) else {
        // 读不懂就答 `BAD`——不猜、不崩。
        return [bcall::BAD];
    };
    let said = match op {
        bcall::REGISTER => match seed {
            // 入口要**是它刚交过来的那一枚**（判据在核心：`probe(entry) == who`）。
            Some(entry) => board.register(name, entry, who).map(|_| ()),
            None => Err(Fail::Denied),
        },
        bcall::UNREGISTER => board.unregister(name, who),
        bcall::LOOKUP => {
            // 查到就**把板上那一份转授给客人**：入口不从报文里走，从会话里走。
            // "查不到"与"授不出去"是两件事，故查的结论优先（`.and`）。
            let mut grant = Ok(());
            board
                .lookup_after(name, |entry| grant = bcall::give(entry, who).map(|_| ()))
                .and(grant)
        }
        // 没见过的动作码：与"这个名字不在板上"同一句话（不另立一格）。
        _ => Err(Fail::Unknown),
    };
    [code(said.err())]
}

/// 失败域 → 答话那一格。
fn code(fail: Option<Fail>) -> u8 {
    match fail {
        None => bcall::OK,
        Some(Fail::Unknown) => bcall::UNKNOWN,
        Some(Fail::Taken) => bcall::TAKEN,
        Some(Fail::Denied) => bcall::DENIED,
        Some(Fail::Full) => bcall::FULL,
    }
}

// ── 客侧（服务域）────────────────────────────────────────────

/// 客侧第一步：装上板那条路。返本端这座码头——问与答都从它走。
///
/// `holder` = 客人认的对端 = **它的生我者**（孔交给它，它再转授给板线程）。
pub fn open(holder: TaskId, ms: usize) -> Result<Quay, Fail> {
    let link = Name::new(LINK).map_err(|_| Fail::Unknown)?;
    let mut quay = Quay::open(holder);
    quay.pair(link, ms).map_err(bcall::map_claim)?;
    Ok(quay)
}

/// 客侧第二步：问一句、取一句答。返答话那一格（[`bcall::OK`] = 板收下了）。
///
/// 注册那一句要把**入口**捎上：它经会话交给板（`Accord` 一份），故写进帧里的是"种在板
/// 表里的那个号"——那个号才是板认得的坐标（两个编号空间不同源，互相拿错正是旧树
/// `[33..41]` 那一格的病）。
pub fn ask(link: &Quay, op: u8, name: Name, entry: PieToken, ms: usize) -> Result<u8, Fail> {
    let at = Name::new(LINK).map_err(|_| Fail::Unknown)?;
    let pier = link.find_pier(at).ok_or(Fail::Unknown)?;
    let seed = match op {
        bcall::REGISTER => {
            // 板是谁**从孔本身读**：客人手里那一格是生我者，不是板（板是转授过去的那位）。
            let board = bcall::peer_of(&pier).ok_or(Fail::Unknown)?;
            Some(bcall::hang_in(entry, board).map_err(|()| Fail::Denied)?)
        }
        _ => None,
    };
    // 孔是单槽：槽里还压着上一条时这一推会**等在门外**（`push` 满则挂），不是错误。
    pier.post(&bcall::pack(op, name, seed))
        .map_err(|()| Fail::Unknown)?;
    let mut reply = [0u8; 1];
    match pier.pull(&mut reply, ms) {
        Ok(1) => Ok(reply[0]),
        _ => Err(Fail::Unknown),
    }
}

/// 客侧第三步：把**板刚授进来的那一枚**从本端表里取出来（`LOOKUP` 的下场）。
///
/// 两格判据：`owner` = **本端**（那扇门是本端开的，板授进来的是它的副本）、`vestor` =
/// **板**（这一份是板交给本端的）。本端自己铸的那一枚 `vestor` 是 0（原始自持没有授与
/// 人），故两者分得开。
///
/// 取到的是表里**最后**授进来的那一枚：一次往返只授一枚，故问过几次就取最后一次的。
pub fn take(link: &Quay) -> Option<PieToken> {
    let at = Name::new(LINK).ok()?;
    let board = bcall::peer_of(&link.find_pier(at)?)?;
    let me = bcall::me();
    let mut index = 0usize;
    let mut found = None;
    loop {
        let (token, _perm, vestor) = mail::collect(index).ok()?;
        // 越界哨兵：这一遍扫完了。
        if token.get() == 0 {
            return found;
        }
        index += 1;
        if vestor == board && bcall::opened_by(token) == Some(me) {
            found = Some(token);
        }
    }
}
