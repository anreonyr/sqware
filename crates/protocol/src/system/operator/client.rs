//! operator::client — **客侧**：一条会话 ＋ 一手对一条原语
//!
//! 三侧分家之后本文件只放**客侧**：**开会话那一手不在这里**——它两侧逐字同构，已按"两台以上
//! 逐字同构 ⇒ 收"抬进 [`crate::communication::session`]；本文件只声明**这条路叫什么**
//! （[`BERTH`]）＋ **这一族的几手**（[`Face`] 那一面 / [`id_of`] / [`entry_of`] 与一手对一条
//! 原语）。两侧共用的图与说明见 [`super`] 的"载体"那一节，帧与记号见 [`crate::system::operator`]。
//!
//! **照实记（`plate` 已下移，task-2 那一刀）**：**上树那一趟**（分目录 → 落门牌 → 查回来 →
//! 按号问名）原先是本文件的一手，因"一个组合动作只有一个实现消费者"搬去
//! `programs/src/driver/context.rs`（`Context::plate`）——本文件因此只剩**问到树的那些第二步**
//! 与那一面便利壳。
//!
//! **一手对一条原语**（`land` / `part` / `find` / `trim` / `list` / `seek` / `name`）：线上与模型
//! 是同一件事的两层，客侧这一层也不再拿一个 `op` 码当参数——问什么形状由函数名说。

use crate::message::Message;
use env::Mark;
use env::Wait;
use env::{Name, PieToken, TaskId};
use runtime::core::port::{self, Access, Policy};
use runtime::env::mail;

use crate::communication::establish::Endpoint;
use crate::communication::sender::Sender;
use crate::communication::session::{Berth, Session};
use crate::system::operator as ocall;
use crate::system::operator::Fail;
use crate::system::operator::frame::{Id, Rule};
use crate::system::operator::{EntryId, Listing, Where};

/// **这条路叫什么**：泊位那一格（`LINK` = `operator`）＋ 问话孔那一格（`ASK_MARK`）。
///
/// 开会话那一手（[`Session::open`]）要它；本层只把这两格交出去，不替调用方开会话
/// ——"调用点直接叫定义处"。
pub const BERTH: Berth = Berth {
    link: Mark::of(crate::system::operator::LINK),
    ask: crate::system::operator::ASK_MARK,
};

/// 门牌那一格声不声明归属（[`land`] 的最后一格）。
///
/// 三台驱动今天都是**公开可查**（[`Rule::Public`]），只在这一格上分家：`uart` 说"这枚读行的
/// 孔是我的"（[`Mine::Yes`]），`rtc` / `router` 不说（[`Mine::No`]）。
#[derive(Clone, Copy)]
pub enum Mine {
    /// 这一格是我的。
    Yes,
    /// 不声明归属。
    No,
}

/// **一面持树者**：一条装好的会话（对端 = 持树者）。
///
/// # 它与下面那些自由函数不是一回事
///
/// ```text
///   Face          对外那一层：调用方只拿 [`Face::of`]，不必自己开会话
///   自由函数      内部那一层：持树者自己（`programs/src/system/operator/server.rs` 的
///                 `Court`）与两面门牌的认领处**手里已经有 `Session`**，再包一层 `Face`
///                 只是把同一份 Session 转一圈 —— 故它们留着，且是 `Face` 的实现体。
/// ```
///
/// **包住的是 [`Session`]，不是门牌**：树不像 principal / coalition 那样"一枚门牌即可"——
/// 它要一条装好的会话（问话孔 + 答话路 + 对端号），故 [`Face::of`] 的入参就是 [`Session`]。
///
/// **它不再往下漏别的**：调用方拿到的只有下面那几个方法（`Endpoint` / `Sender` / `Receiver`
/// 一个都不出）；`Session` 本身是**入参**，不是从这一层问出来的。
pub struct Face {
    session: Session,
}

impl Face {
    /// 把一条装好的会话收成一面（`Session::open(sire, BERTH, wait)` 装它）。
    pub fn of(session: Session) -> Face {
        Face { session }
    }

    /// 对端是谁（持树者的号；读数用）。
    pub fn host(&self) -> TaskId {
        self.session.host
    }

    // 照实记（删掉的一格：这里的 `pub fn session() -> &Session`）：它进 `Face` 时写的理由是
    // "给 `operator/server.rs` 的 `find_face` 用"，而那一位的 `find_face` 只吃 `TaskId`、调
    // `claim(ENTRY_MARK, who, None)`——**根本不构造 `Face`**。独立复核逐处查过：全仓零消费者
    // ⇒ 按"没有读者的格不留在面上"删掉。要哪枚孔**自己在实现侧拿 `Session`**（它本来就持着）。

    /// **上树那一趟**（`plate`）**不在这里**（照实记，task-2 那一刀下的移）。
    ///
    /// 它是**某一种程序的装配 recipe**（驱动族的 `/device` 那一段），按"一个组合动作只有
    /// 一个实现消费者就不强升为协议"的裁定，已**下移到** `programs/src/driver/context.rs`
    /// （那一族唯一的实现消费者是 `Context::enter`；`router` 起手经 `Context` 走同一手）。
    /// 本面因此不再有 `plate` 这一格——它调的四手（`part` / `land` / `find` / `name`）仍是
    /// 下面那几个方法，下移只是换了"组合"的住处。

    /// **分**：在 `at` 那一块 `Pane` 里放一块空 `Pane`；答那一格自己的号。
    pub fn part(&self, at: Where, name: Name, wait: Wait) -> Result<EntryId, u8> {
        part(self.session.talk, &self.session.link, at, name, wait)
    }

    /// **落**：在 `at` 那一块 `Pane` 里给 `name` 贴一枚 `Tile`；答那一格自己的号。
    pub fn land(
        &self,
        at: Where,
        name: Name,
        entry: PieToken,
        rule: Rule<Id, Id>,
        mine: bool,
        wait: Wait,
    ) -> Result<EntryId, u8> {
        land(
            self.session.talk,
            &self.session.link,
            self.session.host,
            at,
            name,
            entry,
            rule,
            mine,
            wait,
        )
    }

    /// **寻**：把那一号背后那一枚 Pie 要过来（返 `(状态, 那一格)`）。
    pub fn find(&self, id: EntryId, wait: Wait) -> Result<(u8, Option<PieToken>), Fail> {
        find(self.session.talk, &self.session.link, id, wait)
    }

    /// **剪**：把那一号剪掉。
    pub fn trim(&self, id: EntryId, wait: Wait) -> Result<u8, Fail> {
        trim(self.session.talk, &self.session.link, id, wait)
    }

    /// **列**：看那一块 `Pane` 里有哪些号。
    pub fn list(&self, at: Where, wait: Wait) -> Result<Listing, u8> {
        list(self.session.talk, &self.session.link, at, wait)
    }

    /// **译**：把一条路译成号。
    pub fn seek(&self, road: &[Name], wait: Wait) -> Result<EntryId, u8> {
        seek(self.session.talk, &self.session.link, road, wait)
    }

    /// **名**：这枚号此刻叫什么。
    pub fn name(&self, id: EntryId, wait: Wait) -> Result<Name, u8> {
        name(self.session.talk, &self.session.link, id, wait)
    }

    /// **名字 → 号**（译不出就重试；额度怎么扣见 [`id_of`]——**不是整趟时限**）。
    pub fn id_of(&self, road: &[Name], wait: Wait) -> Result<EntryId, u8> {
        id_of(&self.session, road, wait)
    }

    /// **名字 → 入口**（译号 ＋ `find`；额度怎么扣见 [`entry_of`]——**不是整趟时限**）。
    pub fn entry_of(&self, road: &[Name], wait: Wait) -> Result<PieToken, u8> {
        entry_of(&self.session, road, wait)
    }
}


/// 译号失败之后、再问之前睡多久（毫秒）。
const RETRY_MS: usize = 1;

/// 沿一条路译成号，**答出剩下的额度**（不是"这次重试用掉了多少"）。
///
/// # 照实记一：额度不是时限，它只按 `RETRY_MS` 扣账
///
/// 每重试一轮只扣 [`RETRY_MS`]，**单次往返自己花掉的时间不计入**——这一层量不到"上一条问了
/// 多久"（`Receiver::recv` 只答收到没收到）。故 `left` 是**扣了账的额度**，不是"还剩多少毫秒"：
/// 持有者若每次都恰在期限内答 `UNKNOWN`，**每一轮最长等掉当时那一刻的 `left`**，而 `left` 一轮
/// 只减 1 ⇒ 累计 ≈ n + (n-1) + … ≈ **n²/2**（`n = 10`、每轮 ~9 ms ⇒ 实耗 ≈ 50 ms）。
/// **这是载体层的口径**，本层不假装能给"整趟时限"。
///
/// # 照实记二：`Forever` 显式保 `Forever`，不落进"很大的毫秒数"
///
/// 从前的写法先把 `millis` 折成 `usize::MAX`、再逐轮 `saturating_sub(RETRY_MS)`：**第一轮是
/// `Forever`，一重试就变成 [`Wait::AtMost`]`(usize::MAX - 1)`**——正是 [`Wait`] 头注要避免的
/// 那一格（"一个很大的毫秒数"被当成永久算进期限）。这里改成**按变体扣账**：`Forever` 扣完还是
/// `Forever`，只有 `AtMost(n)` 才减 `RETRY_MS` ⇒ 那道护栏留在类型上，不再是"实践上等价"。
///
/// # 照实记三：这一格原来答错了另一半
///
/// 从前这一圈只在 [`id_of`] 里，而它答的是**号**——额度到那儿就丢了；[`entry_of`] 于是拿
/// **整份** `millis` 再调一次 [`find`] ⇒ 一次 `entry_of` 最坏要花 **≈ 2n**。收成一手之后答话里
/// 多出"**还剩多少**"这一格，`find` 拿剩下的走 ⇒ **那一半（多跑一趟 `find`）没有了**，最坏从
/// ≈ 2n 收到 ≈ n（`Forever` 那一支没有额度，见照实记一）。
fn road_to_id(session: &Session, road: &[Name], millis: Wait) -> Result<(EntryId, Wait), u8> {
    let mut left = millis;
    loop {
        match seek(session.talk, &session.link, road, left) {
            Ok(id) => return Ok((id, left)),
            // 还留着额度就睡一拍再来：`Forever` 恒真，`AtMost(0)` 是"不再等"⇒ 落到下面原样答码。
            Err(ocall::UNKNOWN) if left != Wait::AtMost(0) => {
                let _ = runtime::env::room::sleep(core::time::Duration::from_millis(RETRY_MS as u64));
                left = match left {
                    // **永久不扣账**（它本来没有额度），也不许被折成 `AtMost(usize::MAX - k)`。
                    Wait::Forever => Wait::Forever,
                    Wait::AtMost(n) => Wait::AtMost(n.saturating_sub(RETRY_MS)),
                };
            }
            Err(code) => return Err(code),
        }
    }
}

/// 沿一条路译成号（名字 → 号），**译不出（`UNKNOWN`）就重试**——那几格可能由别的域落下，
/// 它可能落得比本域晚。
///
/// `millis` 是**额度不是时限**（照实记一在 `road_to_id`）：每一趟把**剩下的**当期限递下去
/// （不是每趟都给满——那样"预算"是假的），而扣账只按 `RETRY_MS`。答号，或答线上那一格码。
/// **这一圈原先在八处各写一遍**（收进 `session` 那一刀，见那边的照实记）。
pub fn id_of(session: &Session, road: &[Name], millis: Wait) -> Result<EntryId, u8> {
    road_to_id(session, road, millis).map(|(id, _left)| id)
}

/// 沿一条路**找到那一枚入口**：译号（同上，带重试）＋ [`find`]。
///
/// 答话码原样答；`find` 那一族的两格失败（没收到 / 推不动）折成 [`ocall::BAD`]。
///
/// **这一刀修掉的是"多跑一趟 `find`"那一半**（照实记三在 `road_to_id`）：译号用掉多少额度，
/// `find` 就只有剩下的那些——不是又拿满一份。
///
/// **但它仍不是"整趟时限"，两处照实写**：
///
/// - **额度不是时限**（照实记一）：往返耗时不计账 ⇒ 持有者每次恰在期限内答 `UNKNOWN` 时，最坏
///   ≈ n²/2（`n = 10`、每轮 ~9 ms ⇒ 实耗 ≈ 50 ms），不是 n；
/// - **连"推得进去"都不保证**：`ask_out` 那一步是 `Sender::send(ask, Wait::Forever)`——孔是
///   单槽，槽里压着未读问话就**等在门外** ⇒ 持有者单槽压着一问时，`entry_of(…, AtMost(1))` 可以
///   在这一格上**无界**等下去。
///
/// 故这一族的 `Wait` 只承诺"**本端愿意等多久**"，不承诺"这一趟一定在多久内回来"。
pub fn entry_of(session: &Session, road: &[Name], millis: Wait) -> Result<PieToken, u8> {
    let (id, left) = road_to_id(session, road, millis)?;
    match find(session.talk, &session.link, id, left) {
        Ok((ocall::OK, Some(entry))) => Ok(entry),
        Ok((code, _)) => Err(code),
        Err(_) => Err(ocall::BAD),
    }
}

// 照实记（`plate` 那一手原先在这里）：**上树那一趟**（分目录 → 落门牌 → 查回来 → 按号问名，
// 五条判据 ＋ 一行读数）已**下移到 `programs/src/driver/context.rs`**，落成
// `Context::plate`（task-2 那一刀）。判据与移的理由写在那边；本层只留它调的四手
// （`part` / `land` / `find` / `name`）与这一族其余第二步。
//
// 为什么搬：`plate` 是**驱动族那一段路的装配 recipe**（`dir` 恒为 `protocol::driver::DIR`），
// 而它唯一的实现消费者是 `Context::enter`——按"一个组合动作只有一个实现消费者就不强升为
// 协议"的裁定，它不该住在协议层；`router` 的起手于是改经 `Context` 走同一手（那边照实记）。

// 照实记（原先这里的三手 `open` / `ask_hole` / `me`）：它们与 `board::client` 那三手**逐字
// 同构**（只差两个记号与各自的失败域），已按"两台以上逐字同构 ⇒ 收"抬进
// [`crate::communication::session`]——开会话那一手归地板。本文件因此只剩**这条路的名字**
// （[`BERTH`]）与这一族那几手。


/// 客侧第二步（内里那一手）：**编好的一问推上去，收一句答**。
///
/// 问话推 `say`（开会话那一手铸的问话孔，持树者读），答话从本端这条树路读（持树者写）。
/// 返**收进来的那一答**（[`ocall::Said`]）——**形状由问的人自己读**（答的四种形状在线上分不开，
/// 见 `Said` 的照实记：他问的是哪一条，他自己知道）。
fn ask_out(
    say: PieToken,
    link: &Endpoint,
    ask: ocall::Req<'_>,
    millis: Wait,
) -> Result<ocall::Said, Fail> {
    // 发：装上、发出去——**一帧＝一条报**（偏移与长度不在这层：字段表与 `Message` 说）。
    // 孔是单槽：槽里还压着上一条时这一推会**等在门外**（`push` 满则挂），不是错误。
    Sender::<ocall::Req<'_>>::from_token(say)
        .send(ask, Wait::Forever)
        .map_err(|_| Fail::Unknown)?;
    // 收：答话走本端这条树路——与板那一族同一个形状（`Receiver::<Union>::from_token(pier.hole()).recv(buf, ..)`）。
    // 缓冲由调用方给：这条树路只有持树者会写 ⇒ 本族那只空缓冲（[`Message::EMPTY`]）就够。
    let mut buf = ocall::Union::EMPTY;
    link.receiver::<ocall::Union>()
        .recv(buf.as_mut(), millis)
        // 两格失败（没收到 / 解不动）在这一侧落同一格：对本端是同一个下一步。
        .map_err(|_| Fail::Unknown)
}

/// 客侧第二步（**落**）：在 `at` 那一块 `Pane` 里给 `name` 贴一枚 `Tile`；答**那一格自己的号**。
///
/// `entry` 是客人手里那一枚：它**经会话交给持树者**（`Accord` 一份）后才进帧——报文里走的
/// 是"种在持树者表里的那个号"，那才是它认得的坐标（交出那一手是 `port::ship`：`R|W` ＋
/// 一格 `VEST`，少它 ⇒ 持树者转授那一步答 `Denied`、看上去像"持树者坏了"）。
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
    link: &Endpoint,
    host: TaskId,
    at: Where,
    name: Name,
    entry: PieToken,
    rule: Rule<Id, Id>,
    mine: bool,
    millis: Wait,
) -> Result<EntryId, u8> {
    let pie = mail::HolePie::from_token(entry);
    let shipped = port::ship(&pie, host, Access::FETCH | Access::STORE, Policy::VEST)
        .map(|to| to.seed())
        .map_err(|_| ocall::BAD)?;
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
    link: &Endpoint,
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
    link: &Endpoint,
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
pub fn trim(say: PieToken, link: &Endpoint, id: EntryId, millis: Wait) -> Result<u8, Fail> {
    Ok(ask_out(say, link, ocall::Req::Trim(id), millis)?.code())
}

/// 客侧第二步（**列**）：看那一块 `Pane` 里有哪些**号**（[`Where::Root`] = 根那一层）。
///
/// 答话不是 [`ocall::OK`] ⇒ `Err(那一格码)`：这一条的答案体是数据，码不能当成功值带回来
/// （对照 [`find`]：那一档的码本身就是答案）。一推一读之间读不动 / 迟了 ⇒ [`ocall::BAD`]。
pub fn list(say: PieToken, link: &Endpoint, at: Where, millis: Wait) -> Result<Listing, u8> {
    ask_out(say, link, ocall::Req::List(at), millis)
        .map_err(|_| ocall::BAD)?
        .list()
}

/// 客侧第二步（**译**）：按一条路问"那一格是几号"——**间接寻址那一手**。
///
/// 答话是号那一形（`[0] status [1 .. 9] 号`）：答话不是 [`ocall::OK`] ⇒ `Err(那一格码)`。
/// 拿到号之后同一条路就不必再念了——其余那几条一律按号走（名字只到这一格为止）。
pub fn seek(say: PieToken, link: &Endpoint, road: &[Name], millis: Wait) -> Result<EntryId, u8> {
    ask_out(say, link, ocall::Req::Road(road), millis)
        .map_err(|_| ocall::BAD)?
        .entry()
}

/// 客侧第二步（**名**）：这枚号此刻叫什么。
///
/// 名字**在答话那一侧**（问话里只有号）——长短由那一帧说。
pub fn name(say: PieToken, link: &Endpoint, id: EntryId, millis: Wait) -> Result<Name, u8> {
    ask_out(say, link, ocall::Req::Name(id), millis)
        .map_err(|_| ocall::BAD)?
        .name()
}

// 照实记（删掉的一处：这一面的 `take`）：它从前在这儿——`find` 只答一格状态，客人于是要
// **扫自己的表**按"来源位是持树者"把刚授进来的那一枚认回来（取满足的里最后一枚）。
// `find` 的答话带上那一格之后（见本文件 `find` 的照实记）它没有读者了，按"机制退了，格也退"
// 删掉。**这是 `vestor` 在扫描里的最后一个读者**：它一走，`mail::pies()` 枚举出来的每一枚
// 就只剩 `owner ＋ mark` 两个事实有人在读，`Collect` 那一格便能收窄（乙′ 的末环）。

// 照实记（原先这里还有两件）：`hear`（收"答话的是谁"）与 `map_establish`（建立那一手的
// 失败域对照表）。前者与 `board::client` 那一份逐字同构 ⇒ 随开会话那一手抬进
// [`crate::communication::session`]；后者只服务那一手、且树这一侧**只有一格**（"它不在是
// 一条判据"，铸不出孔 / 交不出去在这一层是同一件事）⇒ 与它的唯一读者一起退场。

