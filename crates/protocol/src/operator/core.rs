//! operator 的核心 —— **树、七条原语（落 / 分 / 寻 / 剪 / 列 / 译 / 名）、失败域**。
//!
//! 本文件**不碰内核**：判据只有一条可机械检查的纪律——
//!
//! > `core.rs` 里不出现 `runtime::`。
//!
//! 两个外部事实是**注入**的：[`VestedBy`]（那一枚 Pie 还答得出吗）与 [`Unship`]（把我这一份放下）。
//! 于是喂两个假闭包就能把这棵树与七条原语的规矩推理干净，换载体不必重写。
//!
//! # 这批判据住在哪
//!
//! 本文件**不放** `#[cfg(test)]`：`protocol` 是 `[lib] test = false`（riscv 目标上编不出
//! libtest），故这里的 `cfg(test)` 一行都不会被主工作区那几道门编到。用例住
//! `crates/operator-case`——一个**编外**的宿主 crate，把**本文件逐字**编进它的测试靶里；
//! 门口是 `scripts/host.sh`。改核心之前先跑它。
//!
//! # 坐标：号是唯一的直接坐标
//!
//! **名字是间接的**：一条路（`&[Name]`）全系统只有一处用处——[`Operator::seek`] 把路**译成号**。
//! 译完就按号走：`find` / `trim` / `name` 收号，`land` / `part` / `list` 收**容器坐标**
//! [`Where`]（根，或某一号）。名字在答话那一侧还留着一条（[`Operator::name`] 按号答名）。
//!
//! **树今天仍是嵌套树**（`Entry { 号, 名字, 去处 }`，`Node::Pane(Vec<Entry>)`）：号 → 那一格
//! 走**一趟扫**（`look` / `holds` / `take` 三个私有助手）。换"按号排的一张表"那一案今天读不出
//! 差别（树就几十格），而它要 `Entry` 多记一格父、删格要保序、列要扫全表——**留到读数说话**
//! （这一段是照实记：选的是甲案，不是忘了乙案）。

use alloc::vec::Vec;

use env::{Name, PieToken, TaskId};

// ── 结构 ────────────────────────────────────────────────────

/// 一枚条目的**号**：机器用的那一个。
///
/// **裸号**：与 [`PrincipalId`](crate::principal::PrincipalId) / [`CoalitionId`](crate::coalition::CoalitionId)
/// 同形（8 字节小端上线），不同源。线上解码面造得出任何号（[`EntryId::new`]），
/// "这枚号还在不在"由每条读查一次树答出来。
///
/// **没有 `ROOT`**（对照另两种号：那两处的 `ROOT` 都在，这里特意没有）：根不是谁条目里的
/// 一条，故**根没有号**——`EntryId(0)` 是第一个**真格子**（`sys`），不是"没有"。
/// "没有这个号"由 [`Fail::Unknown`] 答，别拿 0 当空。根要当坐标时走 [`Where::Root`]。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct EntryId(usize);

impl EntryId {
    /// 由裸号造一个（线上解码面；已失效的号从这里进来）。
    pub const fn new(raw: usize) -> EntryId {
        EntryId(raw)
    }

    /// 裸号。
    pub const fn get(self) -> usize {
        self.0
    }

    /// 线上的那一格（8 字节小端）。
    pub const fn to_bytes(self) -> [u8; 8] {
        (self.0 as u64).to_le_bytes()
    }

    /// 由线上字节还原（**不校验**：还在不在由核心答）。
    pub const fn from_bytes(bytes: [u8; 8]) -> EntryId {
        EntryId(u64::from_le_bytes(bytes) as usize)
    }
}

/// 一条条目：**号 + 名字 + 去处**。
///
/// 名字只是**一段**（`Name`：定长 32 字节、构造即校验），不是整条路——路那一层只剩
/// [`Operator::seek`] 在用。
///
/// 号是机器的：`part` / `land` 铸一枚，此后**换绑不动号**；`trim` 与 [`Operator::find`]
/// 的剔死让那一条走掉 ⇒ 号随之失效（水位不回收，号不重用）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    id: EntryId,
    name: Name,
    node: Node,
}

/// 去处：一块 [`Node::Pane`]（窗格，还能往里走）或一枚 [`Node::Tile`]（砖，到头了）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// 一块 Pane（窗格）：里面是条目。**还能往里走**。
    Pane(Vec<Entry>),
    /// 一枚 Tile（砖）：到头了，就是内核给的那一枚句柄。
    Tile(PieToken),
}

impl Entry {
    /// 这一条的号。
    pub fn id(&self) -> EntryId {
        self.id
    }

    /// 这一条叫什么（一段）。
    pub fn name(&self) -> Name {
        self.name
    }

    /// 它去哪儿。
    pub fn node(&self) -> &Node {
        &self.node
    }
}

/// **容器坐标**：要动的那一块 `Pane` 在哪。
///
/// 两种报法：**根**，或**某一号**。根必须显式占一格——**根没有号**（见 [`EntryId`]），
/// 所以它既不是"0 号"，也不能拿 `Option` 的空位代替：那两样都会被读成"某个真格子"。
///
/// 它的对立面是 [`Operator::find`] / [`Operator::trim`] / [`Operator::name`] 的形参：
/// 那三条要的是**条目**的号，**根根本递不进来**——这是类型义务，不是运行期检查。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Where {
    /// 根那一层：[`Operator::list`] 列的就是它，`land` / `part` 在它下面立一格。
    Root,
    /// 某一号那一块 `Pane` 里。
    At(EntryId),
}

/// 七条原语会失败在哪一格。**一格对应一个不同的下一步**。
///
/// **没有"名字已被占"那一格**：同名接手一枚 `Tile`、或一块**空的** `Pane`，都是换绑
/// （见 [`Operator::land`] / [`Operator::part`]）；而 owner 归 Principal，Operator 分不出
/// "自己 / 别人"，所以"已占即拒"在这里无处落脚。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    /// 那一号/那一格不在树上 ⇒ 换个名字重来，或者先把中间那一层分出来。
    ///
    /// 三条路都走这一格：**没铸过**、`trim` **剪掉了**、[`Operator::find`] **剔死了**
    /// ——水位只往上走、本层不记墓碑，三者长得一样。空路（根）走 [`Operator::seek`] 时也是这一格。
    Unknown,
    /// 那块 `Pane` 里还有东西，而这一手会**毁掉**里面的 ⇒ 先清空。
    ///
    /// 今天只有两条原语走得到它：[`Operator::land`] 的换绑（要把那块非空 `Pane` 换成砖）与
    /// [`Operator::trim`]（要拿走它）。**`part` 不走这一格**——它要的正是那块 `Pane`，
    /// 已经在就是成了（照实记见 [`Operator::part`] 的注）。
    NonEmpty,
    /// 寻到头是一块 `Pane`，不是一枚 `Tile` ⇒ 改用列，或者往它里面走。
    NotATile,
    /// 那一号不是一块 `Pane`（是一枚 `Tile`）⇒ 走不进去；列的时候则说明"那是枚 `Tile`，没什么可列"。
    NotAPane,
    /// 那一块 `Pane` 已经 [`Operator::PANE_CAP`] 条，装不下；或者一条路超过 [`Operator::ROAD_MAX`]
    /// 段（只有 [`Operator::seek`] 走得到这一格）⇒ 拆层 / 扩容量 / 把路缩短。
    Full,
    /// 那枚 Pie 后面的人没了（探不到）⇒ 重落 / 重寻。**剔掉那一条的同时**答这一格。
    Dead,
}

/// **活性**：那一枚 Pie 还答得出吗？答不出（`None`）= 它后面的人没了。
///
/// 与 `system::board` 同一格（`VestedBy` 的形状照旧）：本正文没有 owner，故这里只取"答得出吗"，
/// 答出来的 `TaskId` 用不到。
///
/// **层**：这一格问的是**门闩的授与人**（谁把这一枚交出去的）——与 `Task::heir`（**我生的
/// 子域**）和 `principal::heir`（**谱系谓词**）同字不同层；内核那两个字段名照旧不动。
pub type VestedBy = fn(PieToken) -> Option<TaskId>;

/// **放下**：把我这一份自释。剪掉或换掉一枚 `Tile` 时用它——不加这一格，那一枚句柄就漏在树里。
pub type Unship = fn(PieToken) -> Result<(), ()>;

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

// ── 树 ──────────────────────────────────────────────────────

/// 一棵命名树：**一个 Operator 管着所有条目**。
///
/// 根 = `root` 那一叠条目；[`Where::Root`] 指的就是它（列根那一层、在根下立一格）。
pub struct Operator {
    root: Vec<Entry>,
    /// 铸号的水位：**只增**（全文件没有一处减它）。
    ///
    /// 剔掉一条不回收号 ⇒ **号不重用**：一枚旧号要么还指着原来那一格，要么指空
    /// （[`Fail::Unknown`]），不会悄悄指到后铸的那一条身上。
    next: usize,
    vested_by: VestedBy,
    unship: Unship,
}

impl Operator {
    /// 一块 `Pane` 里最多几条。条数是策略，容器要有界。
    pub const PANE_CAP: usize = 16;
    /// 一条**路**最多几段——只有 [`Operator::seek`] 用得上它（名字只到那一格，往下一律按号）。
    ///
    /// 注意：**树本身没有深度上限**（`land` / `part` 收的是号，层层往下立不受这条路的长短约束）。
    pub const ROAD_MAX: usize = 8;

    /// 立一棵树：两个注入的机制事实跟着树走——它们对每一条同值，故不必逐个作参数传。
    pub const fn new(vested_by: VestedBy, unship: Unship) -> Operator {
        Operator {
            root: Vec::new(),
            next: 0,
            vested_by,
            unship,
        }
    }

    /// **落**：在 `at` 那一块 `Pane` 里，给 `name` 这一格贴一枚 `Tile`；答**那一格自己的号**。
    ///
    /// 四条判据，一条不多：
    ///
    /// - `at` 那块 `Pane` 得在（号不在 ⇒ [`Fail::Unknown`]）；它是一枚 `Tile` ⇒ [`Fail::NotAPane`]；
    /// - `name` 那一格空着 ⇒ **铸一枚新号**放上去；
    /// - `name` 那一格已占 ⇒ **换绑**：旧的那一枚放下（[`Unship`]）、**号不动**，答原来那一枚号
    ///   ——除非它是一块**非空** `Pane`（⇒ [`Fail::NonEmpty`]：要动它先清空）；
    /// - 那一块 `Pane` 已经有 [`Operator::PANE_CAP`] 条 ⇒ [`Fail::Full`]。
    ///
    /// **答的是号**（不是一格状态）：这是"号出门"那一手——立的人自己知道它立成了几号。
    pub fn land(&mut self, at: Where, name: Name, pie: PieToken) -> Result<EntryId, Fail> {
        self.put(at, name, Node::Tile(pie), Want::Tile)
    }

    /// **分**：在 `at` 那一块 `Pane` 里，给 `name` 这一格放一块 `Pane`；答那一格自己的号。
    ///
    /// 门槛与 [`Operator::land`] 同（容器得在、得是 `Pane`），只有"想要什么"这一格不同：
    ///
    /// - 那一格空着 ⇒ 铸一枚新号，放一块**空的** `Pane`；
    /// - 那一格已经是 `Pane` ⇒ **无事**，答它那个号（**幂等**：它要的正是"这儿是一块 `Pane`"，
    ///   里面有没有东西不管——**非空也是成了**，故 `NonEmpty` 不在它这一列）；
    /// - 那一格是一枚 `Tile` ⇒ 换掉它（旧的那一枚放下），号不动；
    /// - 那一块 `Pane` 已经有 [`Operator::PANE_CAP`] 条 ⇒ [`Fail::Full`]。
    ///
    /// 照实记：这一格原来答 `NonEmpty`（"那块非空 Pane 不许动"）。靶上读数把它顶掉了——
    /// 门牌那五处要的是**父格的号**，而它们的父格（`/device`、`/sys`）第二次上来时本来就非空，
    /// 于是"分"答不了号、落门牌跟着塌（`soak-1790097748-1`）。"非空不许动"那条规矩的正当去处
    /// 是**会毁掉内容**的那两条：`land` 的换绑与 `trim`，它们照旧答 [`Fail::NonEmpty`]。
    pub fn part(&mut self, at: Where, name: Name) -> Result<EntryId, Fail> {
        self.put(at, name, Node::Pane(Vec::new()), Want::Pane)
    }

    /// **寻**：把那一号后面那一枚 Pie 交出去。
    ///
    /// - 号不在树上 ⇒ [`Fail::Unknown`]（剪掉、剔死、从没铸过长得一样）；
    /// - 那一格是一块 `Pane` ⇒ [`Fail::NotATile`]；
    /// - 是一枚 `Tile`：**先探一次**（[`VestedBy`]）——答不出 ⇒ 当场剔掉那一条、放下那一份
    ///   （[`Unship`]），答 [`Fail::Dead`]；答得出 ⇒ 交给 `ship`。
    ///
    /// `ship` 是"交出去"那一手（适配层在这里把 Pie 授给调用方，核心因此不碰内核）——与同文件
    /// 另一个注入事实 [`Unship`] 正好是一式的两半（交出 / 放下）。
    pub fn find(&mut self, id: EntryId, mut ship: impl FnMut(PieToken)) -> Result<(), Fail> {
        let vested_by = self.vested_by;
        let unship = self.unship;
        // 先只读地问一遍（借用到此为止），再决定要不要动树。
        let pie = match Self::look(&self.root, id) {
            None => return Err(Fail::Unknown),
            Some(entry) => match &entry.node {
                Node::Pane(_) => return Err(Fail::NotATile),
                Node::Tile(pie) => *pie,
            },
        };
        if vested_by(pie).is_none() {
            let _ = Self::take(&mut self.root, id);
            let _ = unship(pie);
            return Err(Fail::Dead);
        }
        ship(pie);
        Ok(())
    }

    /// **剪**：把那一号那一条剪掉。
    ///
    /// 那一条得存在（否则 [`Fail::Unknown`]）；是 `Pane` 的话**必须空着**（否则 [`Fail::NonEmpty`]）。
    /// 剪掉一枚 `Tile` 时那一枚放下（[`Unship`]）——它是资源实体的一份引用，不放下就漏水。
    pub fn trim(&mut self, id: EntryId) -> Result<(), Fail> {
        let unship = self.unship;
        let dropped = match Self::look(&self.root, id) {
            None => return Err(Fail::Unknown),
            Some(entry) => match &entry.node {
                Node::Pane(inner) if inner.is_empty() => None,
                Node::Pane(_) => return Err(Fail::NonEmpty),
                Node::Tile(pie) => Some(*pie),
            },
        };
        let _ = Self::take(&mut self.root, id);
        if let Some(pie) = dropped {
            let _ = unship(pie);
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
        let level: &[Entry] = match at {
            Where::Root => &self.root,
            Where::At(id) => match Self::look(&self.root, id) {
                None => return Err(Fail::Unknown),
                Some(entry) => match &entry.node {
                    Node::Pane(inner) => inner,
                    Node::Tile(_) => return Err(Fail::NotAPane),
                },
            },
        };
        Ok(level.iter().map(|e| e.id))
    }

    /// **译**：把一条路**译成那一枚号**——名字只能走到这一格，往下一律按号。
    ///
    /// 从根起按名字一段段走：缺一段 ⇒ [`Fail::Unknown`]；中途那一段是一枚 `Tile` ⇒
    /// [`Fail::NotAPane`]；路超过 [`Operator::ROAD_MAX`] 段 ⇒ [`Fail::Full`]。
    /// 走到头答**那一格自己的号**——故**最后一段是一枚 `Tile` 也行**（那正是门牌那一格：
    /// `/device/uart` 到头就是一枚砖）。
    ///
    /// **空路 ⇒ [`Fail::Unknown`]**（对照 [`Operator::list`]：它空路却能列——列的是根那一层，
    /// 不需要根有号）：**根没有号**，没什么可译。
    ///
    /// 与 `list` 一样**不过问死活**：剔死是 [`Operator::find`] 那一路上的事。
    pub fn seek(&self, road: &[Name]) -> Result<EntryId, Fail> {
        if road.len() > Self::ROAD_MAX {
            return Err(Fail::Full);
        }
        // 空路 ⇒ 根 ⇒ 没有号（`split_last` 那一步就把它挡在这一格）。
        let (last, rest) = road.split_last().ok_or(Fail::Unknown)?;
        let mut level = &self.root;
        for step in rest {
            let entry = level
                .iter()
                .find(|e| e.name == *step)
                .ok_or(Fail::Unknown)?;
            level = match &entry.node {
                Node::Pane(inner) => inner,
                Node::Tile(_) => return Err(Fail::NotAPane),
            };
        }
        let entry = level
            .iter()
            .find(|e| e.name == *last)
            .ok_or(Fail::Unknown)?;
        Ok(entry.id)
    }

    /// **名**：这枚号此刻叫什么。
    ///
    /// **一趟全树扫**（O(全树)、零新状态）：要 O(深度) 就得让 `Entry` 记父路，多一格、
    /// 删格要维护，本刀不取（与上面"树仍是嵌套树"那段照实记同一件事）。
    ///
    /// 号失效（`trim` 剪掉、[`Operator::find`] 剔死）与从来没铸过**长得一样** ⇒ 都答
    /// [`Fail::Unknown`]：水位只往上走，本层不记墓碑，也分不出这两件事。
    pub fn name(&self, id: EntryId) -> Result<Name, Fail> {
        Self::look(&self.root, id)
            .map(|entry| entry.name)
            .ok_or(Fail::Unknown)
    }

    // ── 走路 ────────────────────────────────────────────────

    /// 落 / 分共用的那一手：在 `at` 那一块 `Pane` 里给 `name` 立一格（或换绑那一格）。
    ///
    /// **答那一格自己的号**：换绑不动号，故只有"真铸了一格"才动水位。
    fn put(&mut self, at: Where, name: Name, node: Node, want: Want) -> Result<EntryId, Fail> {
        let unship = self.unship;
        let id = match at {
            Where::Root => Self::put_in(
                &mut self.root,
                None,
                name,
                node,
                want,
                &mut self.next,
                unship,
            )?,
            Where::At(id) => Self::put_in(
                &mut self.root,
                Some(id),
                name,
                node,
                want,
                &mut self.next,
                unship,
            )?,
        };
        Ok(id)
    }

    /// 走到 `target` 那一块 `Pane`（`None` = 根）里，立一格 / 换绑一格。
    ///
    /// **先只读地问一遍**（[`Operator::holds`]）再往下递归：这样这一层的返回值是**答案本身**
    /// （一个号），而不是一个借用——`&mut` 穿过 `iter_mut` 那层循环再返出去是过不了借用检查的。
    fn put_in(
        level: &mut Vec<Entry>,
        target: Option<EntryId>,
        name: Name,
        node: Node,
        want: Want,
        next: &mut usize,
        unship: Unship,
    ) -> Result<EntryId, Fail> {
        let Some(target) = target else {
            return Self::put_here(level, name, node, want, next, unship);
        };
        // 目标就是这一层里的一条 ⇒ 它得是 `Pane`（否则走不进去）。
        if let Some(slot) = level.iter().position(|e| e.id == target) {
            return match &mut level[slot].node {
                Node::Pane(inner) => Self::put_here(inner, name, node, want, next, unship),
                Node::Tile(_) => Err(Fail::NotAPane),
            };
        }
        // 否则往下找：只有"握着它"的那一块才下去。
        for entry in level.iter_mut() {
            if let Node::Pane(inner) = &mut entry.node {
                if Self::holds(inner, target) {
                    return Self::put_in(inner, Some(target), name, node, want, next, unship);
                }
            }
        }
        Err(Fail::Unknown)
    }

    /// 就在这一层里立一格 / 换绑一格（[`Operator::put_in`] 走到地方之后的那一手）。
    fn put_here(
        level: &mut Vec<Entry>,
        name: Name,
        node: Node,
        want: Want,
        next: &mut usize,
        unship: Unship,
    ) -> Result<EntryId, Fail> {
        let fresh = EntryId::new(*next);
        match level.iter().position(|e| e.name == name) {
            Some(slot) => {
                // 已占。**换绑不动号**：那一格还是同一格——答的就是它那个号。
                let id = level[slot].id;
                // **分那边幂等**：想要的就是"这儿是一块 `Pane`"，已经在就是成了（`Want::Pane`）。
                if want == Want::Pane && matches!(level[slot].node, Node::Pane(_)) {
                    return Ok(id);
                }
                let old = match &level[slot].node {
                    Node::Tile(old) => Some(*old),
                    Node::Pane(inner) if inner.is_empty() => None,
                    // 落到这里只可能是 `Want::Tile`：换绑会毁掉那块非空 `Pane` 里的东西。
                    Node::Pane(_) => return Err(Fail::NonEmpty),
                };
                level[slot].node = node;
                if let Some(old) = old {
                    let _ = unship(old);
                }
                Ok(id)
            }
            None => {
                if level.len() >= Self::PANE_CAP {
                    return Err(Fail::Full);
                }
                // **先要位、再落格**：上面那一格管的是**条数**（`PANE_CAP`），这一格管
                // **内存**。少了它，分配失败走的是 `handle_alloc_error`（abort）——而同一句
                // "备不下就如实报"在仓里另外两处都是 `try_reserve → Full`：`Desk::admit`
                // （`programs/src/supervisor/operator/desk.rs`）与 `Ledger::grow`
                // （`crates/protocol/src/operator/ledger.rs`）。**同一句话，三处一个纪律。**
                level.try_reserve(1).map_err(|_| Fail::Full)?;
                level.push(Entry {
                    id: fresh,
                    name,
                    node,
                });
                *next += 1;
                Ok(fresh)
            }
        }
    }

    /// 这一片子树里有没有 `id` 那一条（只读；递归下去的一趟扫）。
    fn holds(level: &[Entry], id: EntryId) -> bool {
        level.iter().any(|entry| {
            entry.id == id
                || match &entry.node {
                    Node::Pane(inner) => Self::holds(inner, id),
                    Node::Tile(_) => false,
                }
        })
    }

    /// 按号找那一条（只读；一趟全树扫）。号不在 ⇒ `None`。
    fn look(level: &[Entry], id: EntryId) -> Option<&Entry> {
        for entry in level {
            if entry.id == id {
                return Some(entry);
            }
            if let Node::Pane(inner) = &entry.node {
                if let Some(found) = Self::look(inner, id) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// 按号把那一条**拿走**（交出来的是一条，不是借用）。
    fn take(level: &mut Vec<Entry>, id: EntryId) -> Option<Entry> {
        if let Some(slot) = level.iter().position(|e| e.id == id) {
            return Some(level.remove(slot));
        }
        for entry in level.iter_mut() {
            if let Node::Pane(inner) = &mut entry.node {
                if Self::holds(inner, id) {
                    if let Some(found) = Self::take(inner, id) {
                        return Some(found);
                    }
                }
            }
        }
        None
    }
}
