//! operator::core — **树那一本账**：一张按号排的表 ＋ 七条线上原语 ＋ 三条给判据的。
//!
//! **照实记（它原先住 `protocol::service::operator::core`）**：那一份的读者只有本域的持树者
//! （`prog-operator` 那一枚线程）——按"协议 = 共享语言"的判据，它属于实现侧。协议那一侧
//! 只留**上线的类型**（`EntryId` / `Where` / `Fail` / `Permit` / `Ruling` 与两条容量）
//! 与客侧几手。
//!
//! 本目录三份：`mod.rs` 树（**两轴的事实都住在砖上**）＋ `judge.rs` 门外那一问 ＋
//! `gate.rs` 裁决折成线上一格。**判定与树同住这一侧**，故 `gate` 那三格线上码与
//! `frame` 的同步断言也搬到这里（见本文件末尾）。
//!
//! **照实记（那本账整本退场）**：归属从前另住一份 `ledger.rs`（`Ledger` / `Line` / `Key` /
//! `fresh`，305 行），它是树的**一层影子**——`Line` 那四格里 `name` 与 `id` 就是 `Slot` 的名字
//! 与下标、`at` 走一步就能重算，只有"谁声明了这一格"是新的；而那一格里记的 `PieToken`
//! 更是砖上那一枚**本身**（`land` 那一趟把同一个值同时交给账与树）。影子因此得靠一次 `fresh`
//! 对账 ＋ **五处**手写销账来维持"账 ⊆ 树"，而"**与砖同生 ⇒ 那条失效面构造上不存在**"正是上一
//! 刀刚立过的判据（那时收掉的是许可那一轴）。影子撤掉：两轴在同一张表上一次读出来。

use alloc::string::String;
use alloc::vec::Vec;

use env::{PieToken, TaskId};

use protocol::communication::establish::{opened_by, vested_by};
use protocol::service::operator::frame::PANE_CAP;
use protocol::service::operator::path::Path;
use protocol::service::operator::{EntryId, Fail, Permit, Where};

// ── 两个子模块 ──────────────────────────────────────────────
pub mod gate;
pub mod judge;

/// **一格**：名字 + 去处。它住在 [`Operator::slots`] 里，**下标就是它的号**。
///
/// 名字只是一段（[`String`]：不长于 255 字节那一格由**编帧**那一刻判），不是整条路——路那一层只剩
/// [`Operator::seek`] 在用。
struct Slot {
    name: String,
    node: Node,
}

/// 去处：一块 [`Node::Pane`]（窗格，还能往里走）或一枚 [`Node::Tile`]（砖，到头了）。
enum Node {
    /// 一块 Pane（窗格）：里面是**孩子的号**（按登记序）。**还能往里走**。
    Pane(Vec<EntryId>),
    /// 一枚 Tile（砖）：到头了，就是内核给的那一枚句柄；**「用」与「改」两轴都住在它上面**。
    ///
    /// **两轴为什么不挂 `Slot`**：① 许可只在 `find` 那一问上被读，而 `find` 只认砖 ⇒ 它本来就
    /// 是"一枚砖的性质"；② 挂 `Slot` 会造出"一块 `Pane` 也有许可 / 也有主人"这两格，而它们
    /// 今天不存在（`part` 分出来的窗格两样都没有）；③ `part` 把砖顶成窗格时两轴**随砖自然没**，
    /// 与从前销那一行账的结果逐字相同，不必另写清理。三条合起来：这两格**写不出来**。
    Tile {
        pie: PieToken,
        permit: Permit,
        /// **这一格归谁改**——「改」那一轴那一句话：`Some(who)` = 当初落牌那一位声明了这一格
        /// 归他改；`None` = 从没声明过归属 ⇒ 谁都能落（`mine = false` 那一路）。
        ///
        /// **为什么记的是"命"不是"身份"**（照实记：这一格从 `f4f0da9` 一直悬着，裁在后来
        /// "两轴分家"那一刀）——三条理由，一条比一条硬：
        ///
        /// 1. **键与护栏必须同级**：这一轴唯一的护栏是"**主人还在不在场**"（[`vested_by`]，
        ///    问的是砖上那一枚还答不答得出），而**封印是按任务来的**（退场钩子封印该域开的
        ///    资源）。若键改成身份，护栏就得问"那**一条身份**还在不在场"——而身份**永不消亡**
        ///    （`principal` 的节点只增不删）⇒ 那条护栏当场失去意义。
        /// 2. **`mine = true` 是一个动作的产物**："**我**落牌那一刻声明这一格归我"——主语是任务。
        /// 3. **今天没有客人要那另一种**（"另一枚 TID 代表同一条身份也能改得动"）。
        ///
        /// **被否**：把这一格换成身份号。代价两条：① 写那一侧要多问一次名册（`claimable` 之前
        /// 先换身份），而"用"那一侧才刚为 [`Permit::Opener`] 破过一次"只问一次"；② 键与护栏
        /// 不同源（理由 1）。换来的是"与用那一轴同键"这点形式上的整齐——不值。
        ///
        /// **它为什么不另带一枚 `PieToken`**（照实记：影子账那一版带，撤了）：护栏那一问要的
        /// 那一枚就是**砖上那一枚**——`land` 那一趟把同一个值同时交给两轴，两者**同生同灭**
        /// （换绑、顶成窗格、剪掉都一起走）⇒ 账上那一份是多余的拷贝。
        owner: Option<TaskId>,
    },
}

// **照实记（三枚注入的函数指针退场）**：这里从前有 `VestedBy`（那一枚还答得出吗）、
// `Unship`（把我这一份放下）、`OpenedBy`（这扇门是谁开的）三个类型别名，外加一格
// `Stamps { vested_by, opened_by }` 把它们成组收着——由 `tree()` 在构造时接上身体。
// 判据一字没改：**身体只有一个**（[`crate::communication::establish`]），
// 而"接上"只是把同一个函数换名字传一圈 ⇒ 别名、`Stamps`、构造点一起撤，用到的地方直接叫。
// 那一格"两枚同型、摆成位置参数会写反"的顾虑随之不存在（不再有位置参数）。

/// **落 / 分想要什么**——两条原语共用 [`Operator::put`] 那一手，差别只在这一格。
///
/// 照实记：`land` 想要一枚砖（那儿要是块**非空** `Pane` 就是 [`Fail::NonEmpty`]：换绑会毁掉
/// 里面那些），`part` 只想要"这儿是一块 `Pane`"（**幂等**：已经在就是成了，里面有没有东西不管）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Want {
    /// 要一枚 `Tile`（[`Operator::land`]）。
    Tile,
    /// 要一块 `Pane`（[`Operator::part`]）。
    Pane,
}

// ── 两把钥匙：「改」那一轴那两问手里各有的凭据 ────────────────

/// **这一格**的两种报法——两种寻址打的是**同一格**（[`Operator::claimable`] 那两问共用它）。
///
/// 这不是"省一条查法"：坐标是 `land` / `part` 那一问手里唯一的凭据（那时号还不存在或不必
/// 知道），而号是 `trim` 那一问手里唯一的凭据（那时坐标早不知道了）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Key {
    /// 坐标：那一块 `Pane` + 那一段名字（`land` / `part` 那一问手里有的）。
    At(Where, String),
    /// 号：条目自己的号（`find` / `trim` 手里有的）。
    Id(EntryId),
}

// ── 树 ──────────────────────────────────────────────────────

/// 一棵命名树：**一个 Operator 管着所有条目**。
///
/// 根 = `root` 那一叠**孩子的号**（[`Where::Root`] 指的就是它）；其余每一格住 `slots` 里，
/// **号就是下标**。两个容器的容量**互不牵连**：`root` 管根那一层，`slots` 管"一共铸过几格"。
pub struct Operator {
    root: Vec<EntryId>,
    /// 一叠格子：**只增不减**（`unlink` 只把那一格置空，不 `pop`）。
    ///
    /// ⇒ **下标即号、号不重用**：一枚旧号要么还指着原来那一格，要么指着墓碑
    /// （[`Fail::Unknown`]），不会悄悄指到后铸的那一格身上。铸号因此**不必另立水位**——水位
    /// 就是 `slots.len()`。**照实记**：原先真存过一份 `next`，它的注自己写着"水位与
    /// `slots.len()` 同值，留着它只为把这条纪律写在一处"——而"两个数必须相等"正是要靠自律
    /// 的那一条；这一处纪律改住在本字段上。
    slots: Vec<Option<Slot>>,
}

impl Operator {
    /// 立一棵树。**机制不进这里**（照实记见文件头）：要探活 / 要开者 / 要放下，
    /// 直接叫 [`crate::communication::establish`] 那三具身体。
    pub const fn new() -> Operator {
        Operator {
            root: Vec::new(),
            slots: Vec::new(),
        }
    }

    /// **落**：在 `at` 那一块 `Pane` 里，给 `name` 这一格贴一枚 `Tile`；答**那一格自己的号**。
    ///
    /// `permit` 是**落牌的人给这一格声明的「用」那一轴**（谁许用这一格），`owner` 是同一趟声明的
    /// **「改」那一轴**（归谁改，`owner` 那一格）——**两轴都与那一枚砖一起落、一起没**：
    /// 换绑覆写它们，`part` 顶成窗格时随之消失，`trim` 剪掉时同理。
    ///
    /// **`owner` 为什么是一格 `Option<TaskId>` 而不是 `mine: bool` ＋ `who`**：那两个是**一轴的
    /// 两半**（"归我" ＋ "我是谁"），四组合里只有两个有意义——`mine = false` 时那个 `who` 谁也不
    /// 读。收成一格之后"没记主人"是一个**值**，不是一句要自己遵守的纪律（同上一刀把三格可选收成
    /// 一格 [`Permit`] 那一手）。线上那一格 `mine` 由适配层折过来（`answer.rs`）。
    ///
    /// 四条判据，一条不多：
    ///
    /// - `at` 那块 `Pane` 得在（号不在 ⇒ [`Fail::Unknown`]）；它是一枚 `Tile` ⇒ [`Fail::NotAPane`]；
    /// - `name` 那一格空着 ⇒ **铸一枚新号**放上去；
    /// - `name` 那一格已占 ⇒ **换绑**：旧的那一枚放下（`mail::release`）、**号不动**，答原来那一枚号
    ///   ——除非它是一块**非空** `Pane`（⇒ [`Fail::NonEmpty`]：要动它先清空）；
    /// - 那一块 `Pane` 已经有 [`PANE_CAP`] 条 ⇒ [`Fail::Full`]。
    ///
    /// **答的是号**（不是一格状态）：这是"号出门"那一手——立的人自己知道它立成了几号。
    pub fn land(
        &mut self,
        at: Where,
        name: String,
        pie: PieToken,
        permit: Permit,
        owner: Option<TaskId>,
    ) -> Result<EntryId, Fail> {
        self.put(at, name, Node::Tile { pie, permit, owner }, Want::Tile)
    }

    /// **分**：在 `at` 那一块 `Pane` 里，给 `name` 这一格放一块 `Pane`；答那一格自己的号。
    ///
    /// 门槛与 [`Operator::land`] 同（容器得在、得是 `Pane`），只有"想要什么"这一格不同：
    ///
    /// - 那一格空着 ⇒ 铸一枚新号，放一块**空的** `Pane`；
    /// - 那一格已经是 `Pane` ⇒ **无事**，答它那个号（**幂等**：它要的正是"这儿是一块 `Pane`"，
    ///   里面有没有东西不管——**非空也是成了**，故 `NonEmpty` 不在它这一列）；
    /// - 那一格是一枚 `Tile` ⇒ 换掉它（旧的那一枚放下），号不动；
    /// - 那一块 `Pane` 已经有 [`PANE_CAP`] 条 ⇒ [`Fail::Full`]。
    ///
    /// 照实记：这一格原来答 `NonEmpty`（"那块非空 Pane 不许动"）。靶上读数把它顶掉了——
    /// 门牌那五处要的是**父格的号**，而它们的父格（`/svc/drv`、`/svc`）第二次上来时本来就非空，
    /// 于是"分"答不了号、落门牌跟着塌（`soak-1790097748-1`）。"非空不许动"那条规矩的正当去处
    /// 是**会毁掉内容**的那两条：`land` 的换绑与 `trim`，它们照旧答 [`Fail::NonEmpty`]。
    pub fn part(&mut self, at: Where, name: String) -> Result<EntryId, Fail> {
        self.put(at, name, Node::Pane(Vec::new()), Want::Pane)
    }

    /// **寻**：把那一号后面那一枚 Pie 交出去。
    ///
    /// - 号不在树上 ⇒ [`Fail::Unknown`]（剪掉、剔死、从没铸过长得一样）；
    /// - 那一格是一块 `Pane` ⇒ [`Fail::NotATile`]；
    /// - 是一枚 `Tile`：**先探一次**（[`vested_by`]）——答不出 ⇒ 当场剔掉那一格、放下那一份
    ///   （`mail::release`），答 [`Fail::Dead`]；答得出 ⇒ 交给 `ship`。
    ///
    /// `ship` 是"交出去"那一手（**回调**：适配层把那一枚授给调用方）——与"放下"正好是
    /// 一式的两半（交出 / 放下）。它留成参数而不直接叫，是因为它带**去授给谁**那一格
    /// （调用方手里那个号），不是无参动作。
    pub fn find(&mut self, id: EntryId, mut ship: impl FnMut(PieToken)) -> Result<(), Fail> {
        // 先只读地问一遍（借用到此为止），再决定要不要动树。
        let pie = match self.slot(id) {
            None => return Err(Fail::Unknown),
            Some(slot) => match &slot.node {
                Node::Pane(_) => return Err(Fail::NotATile),
                Node::Tile { pie, .. } => *pie,
            },
        };
        if vested_by(pie).is_none() {
            let _ = self.unlink(id);
            let _ = runtime::env::mail::release(pie);
            return Err(Fail::Dead);
        }
        ship(pie);
        Ok(())
    }

    /// **开**：第 `id` 格**是谁的门牌**（那一枚句柄的开者）。
    ///
    /// 与 [`Operator::find`] **正相反**：这一条什么都不交出去——只答"是谁"，不把那枚句柄递出去；
    /// 也**不动树**（惰性剔死是 `find` 那一路上的事，这里不顺手销账）。
    ///
    /// 三格，与 `find` 对同一格答**同一批码**（差别只在"活着的砖"那一档：`find` 交出句柄，
    /// 这里答开者）：
    ///
    /// - 号不在树上（墓碑 / 从没铸过）⇒ [`Fail::Unknown`]；
    /// - 那一格是一块 `Pane` ⇒ [`Fail::NotATile`]（**没有开者这一说**）；
    /// - 是一枚 `Tile`，但开者那扇门封印了 ⇒ [`Fail::Dead`]。
    ///
    /// 照实记：`Dead` 那一格是**三因一码**（不是孔 / 不在我表里 / 已封印），与 [`vested_by`] 的
    /// 口径同一句。**不许拆它**——今天没有一位客人分得开这三因，分开就是造三个没人读的格。
    ///
    /// 谁问它：持树者判 [`Permit::Opener`](judge::Permit::Opener) 时，把这一格翻成开者，再拿
    /// 开者去问名册"这一刻代表谁"。故它答的是 **TID**，不是身份号——两件事两个落点。
    pub fn opens(&self, id: EntryId) -> Result<TaskId, Fail> {
        let pie = match self.slot(id) {
            None => return Err(Fail::Unknown),
            Some(slot) => match &slot.node {
                Node::Pane(_) => return Err(Fail::NotATile),
                Node::Tile { pie, .. } => *pie,
            },
        };
        opened_by(pie).ok_or(Fail::Dead)
    }

    /// **剪**：把那一号那一格剪掉。
    ///
    /// 那一格得存在（否则 [`Fail::Unknown`]）；是 `Pane` 的话**必须空着**（否则 [`Fail::NonEmpty`]）。
    /// 剪掉一枚 `Tile` 时那一枚放下（`mail::release`）——它是资源实体的一份引用，不放下就漏水。
    ///
    /// **剪掉的那一槽留成墓碑**（`None`），不 `remove`：号是下标，一移后面全错位。故一枚剪过的
    /// 号从此答 [`Fail::Unknown`]，而**它不会被重新铸出来**（水位只增）。
    pub fn trim(&mut self, id: EntryId) -> Result<(), Fail> {
        let dropped = match self.slot(id) {
            None => return Err(Fail::Unknown),
            Some(slot) => match &slot.node {
                Node::Pane(inner) if inner.is_empty() => None,
                Node::Pane(_) => return Err(Fail::NonEmpty),
                Node::Tile { pie, .. } => Some(*pie),
            },
        };
        let _ = self.unlink(id);
        if let Some(pie) = dropped {
            let _ = runtime::env::mail::release(pie);
        }
        Ok(())
    }

    /// **列**：看那一块 `Pane` 里有哪些**号**（[`Where::Root`] = 根那一层）。
    ///
    /// 号不在 ⇒ [`Fail::Unknown`]；那一号是一枚 `Tile` ⇒ [`Fail::NotAPane`]。**不过问死活**：
    /// 剔死是 [`Operator::find`] 那一路上的事。
    ///
    /// 答的是号、不是名字（机器用号，人用名——名字另问 [`Operator::name`]）。
    /// 顺序是**号序**：pane 里本来是登记序，而号单调 ⇒ 两者一致，不必额外排。
    pub fn list(&self, at: Where) -> Result<impl Iterator<Item = EntryId> + '_, Fail> {
        Ok(self.kids(at)?.iter().copied())
    }

    /// **译**：把一条路**译成那一枚号**——名字只能走到这一格，往下一律按号。
    ///
    /// 从根起按名字一段段走：缺一段 ⇒ [`Fail::Unknown`]；中途那一段是一枚 `Tile` ⇒
    /// [`Fail::NotAPane`]。
    /// 走到头答**那一格自己的号**——故**最后一段是一枚 `Tile` 也行**（那正是门牌那一格：
    /// `/svc/drv/uart/rx` 到头就是一枚砖）。
    ///
    /// **空路 ⇒ [`Fail::Unknown`]**（对照 [`Operator::list`]：它空路却能列——列的是根那一层，
    /// 不需要根有号）：**根没有号**，没什么可译。
    ///
    /// 与 `list` 一样**不过问死活**：剔死是 [`Operator::find`] 那一路上的事。
    /// **"路太长"那一格退了**（照实记）：它从前在这里答 [`Fail::Full`]（段数那一格写得下
    /// 9），而今天一条路是 [`Path`]——最多 [`Path::MAX`] 段，超长根本造不出来 ⇒ 那一格由
    /// `RoadFrame`（`env::wire::Span` 那一手）在上游判成"读不懂"（门答 `BAD`）。这条判据因此删掉，
    /// [`Fail::Full`] 只剩"那一块 `Pane` 满"一个来源。
    pub fn seek(&self, road: &Path) -> Result<EntryId, Fail> {
        // 空路 ⇒ 根 ⇒ 没有号（`parent()` 在根上答 `None`，这一步就把它挡在这一格）。
        let prefix = road.parent().ok_or(Fail::Unknown)?;
        let last = road.file_name().ok_or(Fail::Unknown)?;
        let mut level: &[EntryId] = &self.root;
        for step in prefix.iter() {
            let child = self.child(level, step).ok_or(Fail::Unknown)?;
            level = match &self.slot(child).ok_or(Fail::Unknown)?.node {
                Node::Pane(inner) => inner,
                Node::Tile { .. } => return Err(Fail::NotAPane),
            };
        }
        self.child(level, last).ok_or(Fail::Unknown)
    }

    /// **名**：这枚号此刻叫什么。
    ///
    /// **一趟查表**（照实记：原来是**一趟全树扫**——那时号不是下标；这一版里
    /// [`Operator::slot`] 直接取，故 `name` 与 `find` / `trim` 一样是 O(1)）。
    ///
    /// 号失效（`trim` 剪掉、[`Operator::find`] 剔死）与从来没铸过**长得一样** ⇒ 都答
    /// [`Fail::Unknown`]：表里那一槽是个墓碑，而墓碑不对外说话。
    pub fn name(&self, id: EntryId) -> Result<String, Fail> {
        self.slot(id)
            .map(|slot| slot.name.clone())
            .ok_or(Fail::Unknown)
    }

    /// **这一格许给谁**——「用」那一轴上那一句话。
    ///
    /// [`Permit::Unset`] 盖三种"没有"，而判据只需要"有没有那一句话"这一件事：号不在树上
    /// （墓碑 / 从没铸过）、那一号是一块 `Pane`、以及砖上没记许可。**不是失败**——门外那一问
    /// 没有"我判不了"的余地（判不了要留给问身份那两条边）。
    pub fn permit(&self, id: EntryId) -> Permit {
        match self.slot(id) {
            Some(Slot {
                node: Node::Tile { permit, .. },
                ..
            }) => *permit,
            _ => Permit::Unset,
        }
    }

    /// **这一格归不归 `who` 改**——「改」那一轴由树自己答。
    ///
    /// 四支，每一支一个理由：
    ///
    /// - 那一格**不是砖**（没铸过 / 剪掉 / 剔死 / 是一块 `Pane`）⇒ **可以**：没有砖就没有主人，
    ///   而 `part` 分出来的窗格这一轴压根没有；
    /// - 砖上**没记主人** ⇒ **可以**（从没声明过归属；`mine = false` 落的牌走这一支）；
    /// - 就是他 ⇒ **可以**（主人改自己的格子，包括用 `mine = false` 重绑一次放弃）；
    /// - 主人**不在场**（那一枚答不出）⇒ **可以**——这就是"规矩属于**活着的**主人"。
    ///
    /// 两个门里的那一问（一轴一件事）：
    ///
    /// - **不是"这一格归谁"**（答主人是谁）：没有任何客人问得出它，故那一格不落地；
    /// - **不是"不许改"**：不许是适配层把它翻成线上一格码的事（`answer.rs` 那一侧），核只答
    ///   "归不归"。（同 [`Operator::permit`]：核答那一句话，门前那一问由 `door.rs` 做。）
    ///
    /// **不在这里对账**（照实记：影子账那一版每次查都得问"那一号此刻还是砖吗"并顺手销陈账）：
    /// 今天这一问就是 [`Operator::slot_at`] 那一次查表——**影子没有了，也就没有"对不上"这一格**。
    pub fn claimable(&self, key: Key, who: TaskId) -> bool {
        let Some(Slot {
            node: Node::Tile { pie, owner, .. },
            ..
        }) = self.slot_at(key)
        else {
            return true;
        };
        match owner {
            None => true,
            Some(owner) => *owner == who || vested_by(*pie).is_none(),
        }
    }

    // ── 走路 ────────────────────────────────────────────────

    /// 落 / 分共用的那一手：在 `at` 那一块 `Pane` 里给 `name` 立一格（或换绑那一格）。
    ///
    /// **答那一格自己的号**：换绑不动号，故只有"真铸了一格"才动水位。**不递归**——
    /// `at` 那一块 `Pane` 就是一次 [`Operator::kids`]。
    ///
    /// **先只读地问一遍**（那一格叫什么号），再动手：这样动手那一段只需要一次
    /// `slots[i]` 的可变借用，不必在 children 里穿一层 `&mut`（那正是老一版递归的由头）。
    fn put(&mut self, at: Where, name: String, node: Node, want: Want) -> Result<EntryId, Fail> {
        let existing = self
            .kids(at)?
            .iter()
            .copied()
            .find(|child| self.slot(*child).is_some_and(|slot| slot.name == name));
        match existing {
            Some(id) => {
                // 已占。**换绑不动号**：那一格还是同一格——答的就是它那个号。
                let Some(slot) = self.slots.get_mut(id.get()).and_then(Option::as_mut) else {
                    // 不可能：`kids` 里每一个号都指着一条活槽（那一格不变量）。
                    return Err(Fail::Unknown);
                };
                // **分那边幂等**：想要的就是"这儿是一块 `Pane`"，已经在就是成了（`Want::Pane`）。
                if want == Want::Pane && matches!(slot.node, Node::Pane(_)) {
                    return Ok(id);
                }
                let old = match &slot.node {
                    Node::Tile { pie: old, .. } => Some(*old),
                    Node::Pane(inner) if inner.is_empty() => None,
                    // 落到这里只可能是 `Want::Tile`：换绑会毁掉那块非空 `Pane` 里的东西。
                    Node::Pane(_) => return Err(Fail::NonEmpty),
                };
                slot.node = node;
                if let Some(old) = old {
                    let _ = runtime::env::mail::release(old);
                }
                Ok(id)
            }
            None => {
                if self.kids(at)?.len() >= PANE_CAP {
                    return Err(Fail::Full);
                }
                // **先要位、再落格**：条数那一闸管的是`PANE_CAP`，这两行管**内存**。
                // 少了它们，分配失败走的是 `handle_alloc_error`（abort）——而同一句"备不下就
                // 如实报"在仓里另外一处是 `try_reserve → Full`：`Desk::admit`
                // （`crates/protocol/src/system/desk.rs`）。**同一句话，两处一个纪律**
                // （照实记：原先第三处是那本影子账的 `Ledger::land`，它随影子一起退场了）。
                //
                // 两处都要长：一格住 `slots`，一个号进 `root` 或某个 `Pane` 的 children。
                // 先要位再落格 ⇒ 半路失败**不留半个状态**（下面两处 `push` 都不会再分配）。
                self.slots.try_reserve(1).map_err(|_| Fail::Full)?;
                self.kids_mut(at)?.try_reserve(1).map_err(|_| Fail::Full)?;
                // **号就是这一格的下标**（`slots` 只增不减 ⇒ 号不重用）：水位不另存一份
                // （见 `slots` 那个字段的照实记）。
                let fresh = EntryId::new(self.slots.len());
                self.slots.push(Some(Slot { name, node }));
                self.kids_mut(at)?.push(fresh);
                Ok(fresh)
            }
        }
    }

    /// 这一块 `Pane` 的孩子（`Where::Root` = 根那一层；`At(id)` = 那一格，得是 `Pane`）。
    ///
    /// 三格判据与老一版的走法逐条对齐：号不在 ⇒ [`Fail::Unknown`]；那一格是 `Tile` ⇒
    /// [`Fail::NotAPane`]。
    fn kids(&self, at: Where) -> Result<&[EntryId], Fail> {
        match at {
            Where::Root => Ok(&self.root),
            Where::At(id) => match self.slot(id) {
                None => Err(Fail::Unknown),
                Some(slot) => match &slot.node {
                    Node::Pane(inner) => Ok(inner),
                    Node::Tile { .. } => Err(Fail::NotAPane),
                },
            },
        }
    }

    /// [`Operator::kids`] 的可变那一半（判据同一份）。
    fn kids_mut(&mut self, at: Where) -> Result<&mut Vec<EntryId>, Fail> {
        match at {
            Where::Root => Ok(&mut self.root),
            Where::At(id) => match self.slots.get_mut(id.get()).and_then(Option::as_mut) {
                None => Err(Fail::Unknown),
                Some(slot) => match &mut slot.node {
                    Node::Pane(inner) => Ok(inner),
                    Node::Tile { .. } => Err(Fail::NotAPane),
                },
            },
        }
    }

    /// 按号取那一格（只读）。号不在 / 是墓碑 ⇒ `None`。
    ///
    /// 这一格是"号就是下标"那一句的全部实现：**一趟查表，不递归、不扫树**。
    fn slot(&self, id: EntryId) -> Option<&Slot> {
        self.slots.get(id.get()).and_then(Option::as_ref)
    }

    /// 按 [`Key`] 那两把钥匙取那一格（只读）。两种寻址**打的是同一格**（见 [`Key`]）。
    ///
    /// 坐标那一路走的就是 [`Operator::put`] 头一步那一趟（[`Operator::kids`] ＋
    /// [`Operator::child`]）：`land` / `part` 那一问发生在**动树之前**，故它得自己先走这一趟。
    fn slot_at(&self, key: Key) -> Option<&Slot> {
        match key {
            Key::Id(id) => self.slot(id),
            Key::At(at, name) => self.slot(self.child(self.kids(at).ok()?, name.as_str())?),
        }
    }

    /// 在这一块 `Pane` 的孩子里按名字找那个号（只读一趟扫，最多 `PANE_CAP` 次查表）。
    fn child(&self, level: &[EntryId], name: &str) -> Option<EntryId> {
        level
            .iter()
            .copied()
            .find(|id| self.slot(*id).is_some_and(|slot| slot.name == name))
    }

    /// 按号把那**一格拿走**：槽标成墓碑（`None`），再把它从**某一个**父的 children 里摘掉。
    ///
    /// **不记父那一格**（照实记：这一版特意不加）。理由两条：
    ///
    /// 1. 我们**没有 `..`**（名字只从根往下走），故父不是给"往上走"用的；它唯一的用处是
    ///    "摘自己"——而那一趟扫是 O(槽数)（几十格），与它换来的一份真相（`children` 与
    ///    `parent` 互为逆，得多维护一处）相比不划算；
    /// 2. Linux 的 `dentry->d_parent` 是为 `..` 与 rename 才必须的——我们两样都没有。
    ///
    /// 摘的顺序：先看根那一层，再逐槽看是不是 `Pane`。**先摘自己再扫**：故那一趟扫看不见自己
    /// 这一槽（已是墓碑），不会把自己从自己里摘。
    fn unlink(&mut self, id: EntryId) -> Option<Slot> {
        let taken = self.slots.get_mut(id.get())?.take()?;
        if let Some(at) = self.root.iter().position(|child| *child == id) {
            self.root.remove(at);
            return Some(taken);
        }
        for i in 0..self.slots.len() {
            let Some(Some(one)) = self.slots.get_mut(i) else {
                continue;
            };
            if let Node::Pane(inner) = &mut one.node
                && let Some(at) = inner.iter().position(|child| *child == id)
            {
                inner.remove(at);
                break;
            }
        }
        Some(taken)
    }
}

// ── 同步义务：`gate` 那三格线上码与 `frame` 的对照表 ──────────────
//
// 真正的对照表只有一份（`protocol::service::operator::frame`）；`gate` 为了"不带载体"自己
// 拿了一份，故在这里**编译期**把两者钉住——一漂就编不过。**这一条住这里**：只有这一层同时
// 看得见 `gate` 与 `frame`。
const _: () = {
    use protocol::service::operator::frame;
    assert!(gate::WIRE_OK == frame::OK);
    assert!(gate::WIRE_DENIED == frame::DENIED);
    assert!(gate::WIRE_UNJUDGED == frame::UNJUDGED);
};
