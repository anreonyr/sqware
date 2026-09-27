//! operator::client — **客侧**：一面持树者，以及它的两个柄（[`Pane`] / [`Tile`]）。
//!
//! ```text
//!   Face::of(会话)                session: Session（持有）或 Face::from(&Session)（借用）
//!   Face::root()                  树的根
//!   Face::pane(路) / tile(路)      一条路 → 一块窗格 / 一枚砖（都带重试与额度）
//!   Pane::open / bind / list / trim / name    这一块窗格自己那几手
//!   Pane::tile(路)                            这一块底下的路 → 一枚砖（就地问一次）
//!   Tile::name / pane / token                 这一格自己的读数、另一种读法与那一枚门闩
//! ```
//!
//! **两个柄就是树上那两样东西**：`Pane`（窗格，还能继续分 / 落 / 列）与 `Tile`（砖，到头了，
//! 背后一枚 Pie）。这两个词是**协议自己的词汇**——帧那一侧写着"某一号那一块 `Pane` 里"，失败域
//! 里那两条就叫 [`Fail::NotATile`] / [`Fail::NotAPane`]。不为一个 `u8`、一串号、一枚门闩各造一个
//! 类型——那些各有一手就够（[`Tile::token`] 就是旧 `find`）。
//!
//! **两样东西不需要一个"二选一"的类型**：判据本来就在那两条原语里——`list` 对一枚 `Tile` 答
//! [`Fail::NotAPane`]，`token`（旧 `find`）对一块 `Pane` 答 [`Fail::NotATile`]。故"这一格是
//! 什么"由**你想拿什么**说，不由一次额外的探测说。
//!
//! **有重试的那两格只认路**（[`Face::pane`] / [`Face::tile`]，从根写起）；`Pane` 上的 `open` /
//! `tile` 是"手里已经有一块窗格，就地问一次"那一形。
//!
//! **问话走会话那条路，答话走同一枚树路**：开会话那一手不在这里（两侧逐字同构，已抬进
//! [`crate::communication::session`]）；本文件只声明**这条路叫什么**（[`BERTH`]）。
//!
//! **一手对一条原语**：线上与模型是同一件事的两层，客侧这一层不拿一个 `op` 码当参数——
//! 问什么形状由函数名说。答话那一侧四种形状在线上分不开（`Said` 的照实记），形状由**这一问
//! 是什么**认。
//!
//! **失败域只有一格出口**：`Result<_, Fail>`。裸 `u8` 只活在 [`ocall::Said`] 那几个读法里
//! （wire 那一层），由本文件折成 [`Fail`]——"client → Fail，wire → u8"。

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
/// 开会话那一手（[`Session::open`]）要它；本层只把这两格交出去，不替调用方开会话。
pub const BERTH: Berth = Berth {
    link: Mark::of(crate::system::operator::LINK),
    ask: crate::system::operator::ASK_MARK,
};

/// 门牌那一格声不声明归属（[`Pane::bind`] 的最后一格）。
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
/// **包住的是 [`Session`]，不是门牌**：树不像 principal / coalition 那样"一枚门牌即可"——
/// 它要一条装好的会话（问话孔 + 答话路 + 对端号），故 [`Face::of`] 的入参就是 [`Session`]。
/// 已经持有 `Session`、还要在**同一条会话**上编自己那两枚门牌的地方（`driver/uart/desk.rs`、
/// 两份 `serve_tree`）走 [`Face::from`]——它按值复制那三格（`Endpoint` 是 `Copy`、放下无事）。
///
/// **它不再往下漏别的**：调用方拿到的只有 [`Pane`] / [`Tile`] 与 [`Face::host`]；
/// `Endpoint` / `Sender` / `Receiver` / `Where` 一个都不出。
pub struct Face {
    session: Session,
}

impl Face {
    /// 把一条装好的会话收成一面（`Session::open(sire, BERTH, wait)` 装它）。
    pub fn of(session: Session) -> Self {
        Face { session }
    }

    /// 同一面，**借**一条会话而不是收走它：那三格按值复制（`Endpoint` 无 `Drop`）。
    ///
    /// 留给"会话还要留给别人用"的调用点——本手不改变原会话的归属。
    pub fn from(session: &Session) -> Self {
        Face {
            session: Session {
                link: session.link,
                talk: session.talk,
                host: session.host,
            },
        }
    }

    /// 对端是谁（持树者的号；读数用）。
    pub fn host(&self) -> TaskId {
        self.session.host
    }

    /// 这棵树的**根**（`/`）。
    ///
    /// **它线上是独立那一形**（[`Where::Root`]：`tag = 0`），不是"零号那一格"——根没有号，
    /// 而零号是真格子（`/sys`）。故柄里存的是 [`Where`]，不是一枚 [`EntryId`]。
    pub fn root(&self) -> Pane<'_> {
        Pane {
            face: self,
            at: Where::Root,
        }
    }

    /// 一条**从根出发**的路 → 一块窗格（带重试与额度：路那几格可能由别的域落下）。
    ///
    /// 那一格是一枚砖 ⇒ [`Fail::NotAPane`]（`list` 自己那一问就判了）。
    pub fn pane(&self, road: &[Name], wait: Wait) -> Result<Pane<'_>, Fail> {
        let (id, _left) = road_to_id(&self.session, road, wait)?;
        Pane::at(self, id, wait)
    }

    /// 一条**从根出发**的路 → 一格（带重试与额度：路那几格可能由别的域落下）。
    ///
    /// **它只译号，不顺手取那一枚门闩**：要门闩的是 [`Tile::token`]，那是另一问
    /// （`find` 会动树、还会把那一枚授进来）。多问一趟既费一次往返、又会让"路译得出"这一件
    /// 已经成的事背着另一个失败。
    ///
    /// **四面入口的分工**（`Face` 两格带重试，`Pane` 两格不带）：
    ///
    /// | 入口 | 起点 | 重试 |
    /// |---|---|---|
    /// | [`Face::pane`] / [`Face::tile`] | 根 | **有**（译不出就睡一拍再问，额度 [`RETRY_MS`]） |
    /// | [`Pane::open`] / [`Pane::tile`] | 一块窗格 / 就地问一次 | 无（译不出就答那一格失败） |
    ///
    /// 带重试那两格是"门牌/格子由别的域落下、本域可能比它先起"那一形；不带那两格是
    /// "我手里已经有一条好路"那一形。要哪一形由调用点的处境说，不由默认值兜。
    pub fn tile(&self, road: &[Name], wait: Wait) -> Result<Tile<'_>, Fail> {
        let id = id_of(&self.session, road, wait)?;
        Ok(Tile { face: self, id })
    }
}

/// **一块窗格**：**哪一个容器**是固定下来的宾语，那几手不再重复传它。
///
/// 它能继续分 / 落 / 列——正是"一个值决定后续操作的宾语"那一格，故给它一个柄；一枚砖只需
/// 一枚号 ＋ 取那一枚门闩（见 [`Tile`]）。
///
/// 里面存的是 [`Where`]：**根与"某一号"是线上两形**，柄必须两形都表达得出（[`Face::root`]）。
pub struct Pane<'a> {
    face: &'a Face,
    at: Where,
}

impl<'a> Pane<'a> {
    /// 由一格造柄（帧那一侧答出来的号）。
    fn of(face: &'a Face, id: EntryId) -> Pane<'a> {
        Pane {
            face,
            at: Where::At(id),
        }
    }

    /// 按**窗格**读一格：`list` 它一下——`list` 的判据正是"这一号是一块 `Pane`"
    /// （是一枚砖 ⇒ [`Fail::NotAPane`]；号不在 ⇒ [`Fail::Unknown`]）。
    ///
    /// **它是"把一个已有的号读成窗格"那一格**：[`Face::pane`] 与 [`Pane::tile`] 的落点；
    /// [`Pane::open`] 不走它（`part` 那一问自己就答"这一格是不是窗格"，不必再多问一趟）。
    fn at(face: &'a Face, id: EntryId, wait: Wait) -> Result<Pane<'a>, Fail> {
        ask_out(
            face.session.talk,
            &face.session.link,
            ocall::Req::List(Where::At(id)),
            wait,
        )?
        .list()
        .map_err(map_code)?;
        Ok(Pane::of(face, id))
    }

    /// 这一格是几（**根没有号** ⇒ `EntryId::new(0)`；读数用）。
    ///
    /// 根与零号是两件事（零号是真格子 `/sys`），故这一手在根上答的是那个哨兵值；柄内部存的是
    /// [`Where`]，而 [`Where`] 是帧那一侧的事——**它不出这一面**。
    pub fn id(&self) -> EntryId {
        match self.at {
            Where::Root => EntryId::new(0),
            Where::At(id) => id,
        }
    }

    /// **分**：在这一块里给 `name` 放一块空窗格；答那一格自己的号。
    ///
    /// **幂等**：那一格已经是窗格就答它那个号（里面有没有东西不管）；是一枚砖 ⇒
    /// [`Fail::NotAPane`]；装不下 ⇒ [`Fail::Full`]。
    ///
    /// **它不再补问一趟**：`part` 那一问自己就答"这一格是不是窗格"，再多发一次 `list` 只会
    /// 多一次往返（而多出来那一问的失败会把已经成的 `part` 说成失败——持树者一枚线程，这一格
    /// 是量得出来的代价）。
    pub fn open(&self, name: Name, wait: Wait) -> Result<Pane<'_>, Fail> {
        let said = ask_out(
            self.face.session.talk,
            &self.face.session.link,
            ocall::Req::Part {
                at: self.at,
                name,
            },
            wait,
        )?;
        let id = said.entry().map_err(map_code)?;
        Ok(Pane::of(self.face, id))
    }

    /// **落**：在这一块里给 `name` 贴一枚 `Tile`；答那一格自己的号。
    ///
    /// 五格各是这一格的值：**哪一块窗格**（柄）＋ **叫什么**（`name`）＋ **那一枚**（`e`）＋
    /// **两轴条件**（[`Rule`] 用 / [`Mine`] 改）。不另立一个 struct——那不是语义，是线上那一
    /// 帧的别名。
    ///
    /// `e` 是客人手里那一枚：它**经会话交给持树者**（`Accord` 一份）后才进帧——报文里走的是
    /// "种在持树者表里的那个号"，那才是它认得的坐标。少交一次 ⇒ 持树者转授那一步答
    /// `Denied`、看上去像"持树者坏了"。
    ///
    /// **超时不等于没落**：那一问迟到也折 [`Fail::Unknown`]，而**砖可能照样到了**——要确定性
    /// 就得再问一次（[`Pane::name`] 查得到就是落上了）。
    pub fn bind(
        &self,
        name: Name,
        e: PieToken,
        rule: Rule<Id, Id>,
        mine: Mine,
        wait: Wait,
    ) -> Result<Tile<'_>, Fail> {
        let pie = mail::HolePie::from_token(e);
        let shipped = port::ship(&pie, self.face.session.host, Access::FETCH | Access::STORE, Policy::VEST)
            .map(|to| to.seed())
            .map_err(|_| Fail::Unknown)?;
        let said = ask_out(
            self.face.session.talk,
            &self.face.session.link,
            ocall::Req::Land {
                at: self.at,
                name,
                entry: shipped,
                rule,
                // **归不归自己**是语义；编成那一格 bit 只在这一句（`Mine` 不中途降成 `bool`）。
                mine: matches!(mine, Mine::Yes),
            },
            wait,
        )?;
        let id = said.entry().map_err(map_code)?;
        Ok(Tile {
            face: self.face,
            id,
        })
    }

    /// **列**：这一块里有哪些号（[`Face::root`] = 根那一层）。
    ///
    /// 答的是那一串号（[`Listing`] 是定长值、不是借来的迭代器——它自带 `iter`）。
    pub fn list(&self, wait: Wait) -> Result<Listing, Fail> {
        let said = ask_out(
            self.face.session.talk,
            &self.face.session.link,
            ocall::Req::List(self.at),
            wait,
        )?;
        said.list().map_err(map_code)
    }

    /// **剪**：把 `e` 那一号剪掉。
    pub fn trim(&self, e: EntryId, wait: Wait) -> Result<(), Fail> {
        ask_out(
            self.face.session.talk,
            &self.face.session.link,
            ocall::Req::Trim(e),
            wait,
        )
        .map(|_said| ())
    }

    /// **名**：`e` 那一号此刻叫什么。
    pub fn name(&self, e: EntryId, wait: Wait) -> Result<Name, Fail> {
        let said = ask_out(
            self.face.session.talk,
            &self.face.session.link,
            ocall::Req::Name(e),
            wait,
        )?;
        said.name().map_err(map_code)
    }

    /// **一条路** → 一枚砖：只译号（`seek`），**不动树、不补问**——编号由 [`Tile::token`] 认。
    ///
    /// **路从根写起，不是相对这一块**：`Road` 那一帧只带段列表，持树者从**根**解（正文明写
    /// "名字只到 `seek` 这一格"）。故这一手在哪一块窗格上叫都一样——它收 `&self` 只为不另造一
    /// 个自由入口；柄与这条路无关。
    ///
    /// **它不带重试**（译不出就答那一格失败）：要"译不出就再问"的额度语义走 [`Face::tile`]。
    /// 这一手是"就地问一次"的那一形。
    ///
    /// **它不做窗格那一判**：要窗格走 [`Face::pane`] / [`Pane::open`]——**判据就是 `list`**。
    /// 这里刻意不补那一问：补了既多一次往返，又会把"这一格是砖"这一件正常的事说成失败
    /// （一枚 `Tile` 对 `list` 答 [`Fail::NotAPane`]）。
    pub fn tile(&self, road: &[Name], wait: Wait) -> Result<Tile<'_>, Fail> {
        let said = ask_out(
            self.face.session.talk,
            &self.face.session.link,
            ocall::Req::Road(road),
            wait,
        )?;
        let id = said.entry().map_err(map_code)?;
        Ok(Tile {
            face: self.face,
            id,
        })
    }
}

/// **一枚砖**：`EntryId` 是固定下来的宾语，那一枚门闩是它背后的东西（到头了）。
///
/// 它与 [`Pane`] 是**同一格的两个方向**，不是一个"二选一"的包装：想往里走就 [`Tile::pane`]
/// （`list` 判），想拿那一枚就 [`Tile::token`]（`find` 判）。
pub struct Tile<'a> {
    face: &'a Face,
    id: EntryId,
}

impl Tile<'_> {
    /// 这一格是几（读数用；**跨调用比较的只有这个值**）。
    pub fn id(&self) -> EntryId {
        self.id
    }

    /// 这一格此刻叫什么。
    pub fn name(&self, wait: Wait) -> Result<Name, Fail> {
        let said = ask_out(
            self.face.session.talk,
            &self.face.session.link,
            ocall::Req::Name(self.id),
            wait,
        )?;
        said.name().map_err(map_code)
    }

    /// 按**窗格**读它：是窗格 ⇒ 继续往里走；是一枚砖 ⇒ [`Fail::NotAPane`]。
    pub fn pane(&self, wait: Wait) -> Result<Pane<'_>, Fail> {
        Pane::at(self.face, self.id, wait)
    }

    /// 按**砖**读它：把那一号背后那一枚 Pie 要过来。
    ///
    /// 那一枚**经会话授进本端表**，而**它在本端表里的号随这条答话回来**（[`ocall::Union::Seed`]），
    /// 故客人不必再扫表。寻到头是窗格 ⇒ [`Fail::NotATile`]。
    pub fn token(self, wait: Wait) -> Result<PieToken, Fail> {
        let said = ask_out(
            self.face.session.talk,
            &self.face.session.link,
            ocall::Req::Find(self.id),
            wait,
        )?;
        said.seed().map_err(map_code)
    }
}

// ── 实现原语（**不是第二套对外 API**）────────────────────────
//
// 它们是 [`Pane`] / [`Tile`] 那几个方法的实现体：那一层只做"补上宾语 + 转发 + 解释返回值"。
// 只借 `&Session`（不是 `&Face`）的那几处调用点（`programs/src/driver/context.rs::line` 那类
// 手里有会话、又拿不出 `Face` 所有权的地方）直接叫它们。

/// 译号失败之后、再问之前睡多久（毫秒）。
const RETRY_MS: usize = 1;

/// 客侧第二步（内里那一手）：**编好的一问推上去，收一句答**。
///
/// 问话推 `say`（开会话那一手铸的问话孔，持树者读），答话从本端这条树路读（持树者写）。
/// 返**收进来的那一答**（[`ocall::Said`]）——**形状由问的人自己读**（答的四种形状在线上分不开，
/// 见 `Said` 的照实记：他问的是哪一条，他自己知道）。
fn ask_out(
    say: PieToken,
    link: &Endpoint,
    ask: ocall::Req<'_>,
    wait: Wait,
) -> Result<ocall::Said, Fail> {
    // 发：装上、发出去——**一帧＝一条报**（偏移与长度不在这层：字段表与 `Message` 说）。
    // 孔是单槽：槽里还压着上一条时这一推会**等在门外**（`push` 满则挂），不是错误。
    Sender::<ocall::Req<'_>>::from_token(say)
        .send(ask, Wait::Forever)
        .map_err(|_| Fail::Unknown)?;
    // 收：答话走本端这条树路——缓冲由调用方给：这条树路只有持树者会写 ⇒ 本族那只空缓冲就够。
    let mut buf = ocall::Union::EMPTY;
    link.receiver::<ocall::Union>()
        .recv(buf.as_mut(), wait)
        // 两格失败（没收到 / 解不动）在这一侧落同一格：对本端是同一个下一步。
        .map_err(|_| Fail::Unknown)
}

/// 沿一条路译成号，**答出剩下的额度**（不是"这次重试用掉了多少"）。
///
/// **额度不是时限**：每重试一轮只扣 [`RETRY_MS`]，**单次往返自己花掉的时间不计入**——这一层
/// 量不到"上一条问了多久"（`Receiver::recv` 只答收到没收到）。故 `left` 是**扣了账的额度**：
/// 持有者若每次都恰在期限内答 `UNKNOWN`，每一轮最长等掉当时那一刻的 `left`，而 `left` 一轮只
/// 减 1 ⇒ 累计最坏 ≈ n²/2（`n = 10`、每轮 ~9 ms ⇒ 实耗 ≈ 50 ms）。这是载体层的口径，本层
/// 不假装能给"整趟时限"。
///
/// **`Forever` 扣完还是 `Forever`**（按变体扣账，不折成"很大的毫秒数"）：那道护栏留在类型上，
/// 不是"实践上等价"。
fn road_to_id(session: &Session, road: &[Name], wait: Wait) -> Result<(EntryId, Wait), Fail> {
    let mut left = wait;
    loop {
        match route(session.talk, &session.link, road, left) {
            Ok(id) => return Ok((id, left)),
            // 还留着额度就睡一拍再来：`Forever` 恒真，`AtMost(0)` 是"不再等"⇒ 落下面原样答码。
            Err(Fail::Unknown) if left != Wait::AtMost(0) => {
                let _ = runtime::env::room::sleep(core::time::Duration::from_millis(RETRY_MS as u64));
                left = match left {
                    // **永久不扣账**（它本来没有额度），也不许被折成 `AtMost(usize::MAX - k)`。
                    Wait::Forever => Wait::Forever,
                    Wait::AtMost(n) => Wait::AtMost(n.saturating_sub(RETRY_MS)),
                };
            }
            Err(fail) => return Err(fail),
        }
    }
}

/// 沿一条路译成号（名字 → 号），**译不出（`UNKNOWN`）就重试**——那几格可能由别的域落下，
/// 它可能落得比本域晚。答号，或答线上那一格折出来的失败。
fn id_of(session: &Session, road: &[Name], wait: Wait) -> Result<EntryId, Fail> {
    road_to_id(session, road, wait).map(|(id, _left)| id)
}

/// 沿一条路**找到那一枚入口**那条腿的额度口径，两句照实写在这一处（[`Tile::token`] 走的是
/// 同一条腿）：
///
/// - **这一格修掉的是"多跑一趟"**：译号用掉多少额度，取门闩就只有剩下的那些——不是又拿满一份；
/// - **但它仍不是"整趟时限"**：额度不是时限（见 [`road_to_id`]）；连"推得进去"都不保证
///   ——`ask_out` 那一步是 `Sender::send(ask, Wait::Forever)`，孔是单槽，槽里压着未读问话就
///   **等在门外**。故这一族的 `Wait` 只承诺"**本端愿意等多久**"。
///
/// 两条腿今天各自只由一个读者走：[`road_to_id`] 由 [`Face::room`] / [`Face::entry`] 走，
/// 取门闩那一问由 [`Tile::token`] 走——故没有一处"合起来算额度"的函数再留在这一层。

/// 客侧第二步（**译**）：按一条路问"那一格是几号"——**间接寻址那一手**。
///
/// 拿到号之后同一条路就不必再念了——其余那几条一律按号走（名字只到这一格为止）。
fn route(
    say: PieToken,
    link: &Endpoint,
    road: &[Name],
    wait: Wait,
) -> Result<EntryId, Fail> {
    let said = ask_out(say, link, ocall::Req::Road(road), wait)?;
    said.entry().map_err(map_code)
}

/// 线上那一格码 → 失败域；表外（含 `BAD`）折 [`Fail::Unknown`]。
///
/// 这是**唯一**一处 `u8 → Fail`：`Said` 那几个读法答的是 wire 那一层的码，本族在客侧把它们
/// 收进语义那一格。
fn map_code(code: u8) -> Fail {
    ocall::code_to_fail(code).unwrap_or(Fail::Unknown)
}
