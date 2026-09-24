//! operator 的核心 —— **树、八条原语（落 / 分 / 寻 / 剪 / 列 / 译 / 名 / 开）、失败域**。
//!
//! 本文件**不碰内核**——这条纪律现在由 **crate 边界**管着（`contract` 全树不碰内核那一层，
//! 见本 crate 的头注），故这里不再复述。
//!
//! 外部事实是**注入**的：两枚戳子 [`Stamps`]（那一枚 Pie 还答得出吗、这扇门是谁开的）与
//! [`Unship`]（把我这一份放下）。于是喂三个假闭包就能把这棵树与八条原语的规矩推理干净，
//! 换载体不必重写。
//!
//! 照实记：**第八条（[`Operator::opens`]）不上线**——它没有动作码，是持树者自己判
//! [`Rule::Opens`](super::judge::Rule::Opens) 时问的一句（"这一格是谁的门牌"）。对外那一族
//! 仍是七条（见 [`super::mod`] 那两张表）。
//!
//! # 这批判据住在哪
//!
//! 本文件**不放** `#[cfg(test)]`：`protocol` 是 `[lib] test = false`（riscv 目标上编不出
//! libtest），故这里的 `cfg(test)` 一行都不会被主工作区那几道门编到。用例住
//! `protocol-case` 的 `operator` 靶——一个**编外**的宿主 crate，把**本文件逐字**编进它的测试靶里；
//! 门口是 `crates/gate/tests/host.rs`。改核心之前先跑它。
//!
//! # 坐标：号是唯一的直接坐标
//!
//! **名字是间接的**：一条路（`&[Name]`）全系统只有一处用处——[`Operator::seek`] 把路**译成号**。
//! 译完就按号走：`find` / `trim` / `name` 收号，`land` / `part` / `list` 收**容器坐标**
//! [`Where`]（根，或某一号）。名字在答话那一侧还留着一条（[`Operator::name`] 按号答名）。
//!
//! # 树：**号就是下标**（照实记：那一案原来记着"留到读数说话"，读数说话了）
//!
//! 本文件原来是一棵**嵌套树**（`Entry { 号, 名字, 去处 }`，`Node::Pane(Vec<Entry>)`），
//! 号 → 那一格走**按深度递归**的一趟扫（`look` / `holds` / `take` 三个私有助手）。头注当时
//! 就把"按号排的一张表"那一案记成**留到读数说话**，而读数来了：
//!
//! > `prog-probe-deep` 一层层往下 `part`，**持树者自己死在第 117 层**——四条助手按深度递归，
//! > 而一台域的栈是 `TASK_STACK_SIZE`（16 KiB）；广度有闸（[`Operator::PANE_CAP`]）、一条路
//! > 有闸（[`Operator::ROAD_MAX`]）、**深度一个闸都没有**。死法不是"答一格负码"，是
//! > `user fault killed`：**命名空间整个消失**。
//! >
//! > **照实记（那台探针按用户裁定删了，这四行读数留着）**：`probe-deep` 连同 `SQWARE_ROOT=fair`
//! > 那一景一并删了——它作为**深度**那一格的证客，职责已由宿主门 `protocol-case` 的
//! > `a_deep_chain_does_not_need_the_call_stack` 承担；而它作为**公平**那一格的客人，本来就是
//! > 它被删的理由（见 `crates/gate/src/lib.rs` 里 `Scenario` 那段照实记）。这一段是这一刀改法的
//! > 依据，故不从注里撤。
//!
//! 于是这一版把**号做成表里的下标**：
//!
//! ```text
//!   slots: Vec<Option<Slot>>     号 i ↔ slots[i]；None = 墓碑（剪掉 / 剔死留下的坑）
//!   root:  Vec<EntryId>          根仍然**没有号**（它不是一个槽）
//!   Slot::Pane(Vec<EntryId>)     窗格里装的是**孩子的号**，按登记序
//! ```
//!
//! 四条递归助手一并消失，换成三个平函数（[`Operator::slot`] / [`Operator::kids`] /
//! [`Operator::unlink`]）：**取一格是一趟查表，去一格是从父的 children 里摘一号**。深度不再是
//! 调用栈上的东西——它只决定"你建了多少格"，而那是**容量**问题（与 `PANE_CAP` / `try_reserve`
//! / [`Fail::Full`] 同一条线），与 seL4 的"能力地址是一个整数"、DNS 的"整名 ≤ 255 字节"
//! 同一格。
//!
//! **代价照实记**：`trim` 只把那一槽标成 `None`（**不能** `Vec::remove`——号是下标，一移后面
//! 全错位），故铸过的号永远占一格坑。这一格**不给手拍的上限**：分配失败如实答 [`Fail::Full`]
//! （上一刀在宿主靶上量过那条判据有牙），而"`part` + `trim` 循环能把槽表单调整长"是本文件的
//! 一条**已知边界**（不给手拍的上限，失败答 [`Fail::Full`]）。另一笔：`children ↔ 槽` 从此是**两条真相**（谁的孩子
//! 里有我 / 我在哪个槽），"一格只有一个父"由每条写原语维护，不再是构造性事实。

use alloc::vec::Vec;

use env::{Name, PieToken, TaskId};

use crate::id::Id;

// ── 结构 ────────────────────────────────────────────────────

/// 一枚条目的**号**：机器用的那一个。
///
/// **裸号**：与 [`PrincipalId`](crate::system::principal::core::PrincipalId) / [`CoalitionId`](crate::system::coalition::core::CoalitionId)
/// 同形（8 字节小端上线），不同源。线上解码面造得出任何号（[`EntryId::new`]），
/// "这枚号还在不在"由每条读**查一次表**答出来。
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
}

impl Id for EntryId {
    fn new(raw: usize) -> EntryId {
        EntryId::new(raw)
    }

    fn get(self) -> usize {
        EntryId::get(self)
    }
}

/// **一格**：名字 + 去处。它住在 [`Operator::slots`] 里，**下标就是它的号**。
///
/// 名字只是一段（`Name`：定长 32 字节、构造即校验），不是整条路——路那一层只剩
/// [`Operator::seek`] 在用。
struct Slot {
    name: Name,
    node: Node,
}

/// 去处：一块 [`Node::Pane`]（窗格，还能往里走）或一枚 [`Node::Tile`]（砖，到头了）。
enum Node {
    /// 一块 Pane（窗格）：里面是**孩子的号**（按登记序）。**还能往里走**。
    Pane(Vec<EntryId>),
    /// 一枚 Tile（砖）：到头了，就是内核给的那一枚句柄。
    Tile(PieToken),
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

/// 八条原语会失败在哪一格。**一格对应一个不同的下一步**。
///
/// **没有"名字已被占"那一格**：同名接手一枚 `Tile`、或一块**空的** `Pane`，都是换绑
/// （见 [`Operator::land`] / [`Operator::part`]）；而 owner 归 Principal，Operator 分不出
/// "自己 / 别人"，所以"已占即拒"在这里无处落脚。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    /// 那一号/那一格不在树上 ⇒ 换个名字重来，或者先把中间那一层分出来。
    ///
    /// 三条路都走这一格：**没铸过**、`trim` **剪掉了**、[`Operator::find`] **剔死了**
    /// ——表里留着一个墓碑（`None`），但墓碑**不对外答"我这儿死过"**：三条路长得一样。
    /// 空路（根）走 [`Operator::seek`] 时也是这一格。
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

/// **这扇门是谁开的**：内核 `Reserve` 第二格（`session::call::opened_by`）。
///
/// 与 [`VestedBy`] **同一个类型、不同一句话**——故两枚戳子收在一格里（[`Stamps`]）：谁写反了
/// **编不过**（照实记：这两枚放成位置参数时是同一个类型，写反照样编过；`session` 那一族认
/// `owner` 还是 `grantor` 实测栽过一次，同族的坑不再留）。
///
/// **"答不出"这一格里就有"那扇门封印了"**：开者一退场，它开的门随之封印 ⇒ 答 `None`。
/// 故 `None` 只读作"没有那一位"，不必再问第二个问题。
pub type OpenedBy = fn(PieToken) -> Option<TaskId>;

/// 树要的**两枚戳子**（内核在开门那一刻盖上去的）：这枚还答得出吗 / 这扇门是谁开的。
///
/// 两枚对每一条同值，故跟着树走，不必逐个作参数传（[`Operator::new`] 收一格）。
/// [`Unship`] 不在这里：那两个是**问题**，它是一次**动作**。
#[derive(Clone, Copy)]
pub struct Stamps {
    /// 这一枚还答得出吗（活性）——[`Operator::find`] 的惰性剔死问它。
    pub vested_by: VestedBy,
    /// 这扇门是谁开的——[`Operator::opens`] 问它。
    pub opened_by: OpenedBy,
}

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
    /// 两枚注入的戳子（[`Stamps`]，对每一条同值）。
    stamps: Stamps,
    /// 一次注入的动作（[`Unship`]）。
    unship: Unship,
}

impl Operator {
    /// 一块 `Pane` 里最多几条。条数是策略，容器要有界。
    pub const PANE_CAP: usize = 16;
    /// 一条**路**最多几段——只有 [`Operator::seek`] 用得上它（名字只到那一格，往下一律按号）。
    ///
    /// 注意：**树的深度不受这条路的长短约束**（`land` / `part` 收的是号，层层往下立与路无关）；
    /// 而自从号成了表里的下标，**深度也不再吃调用栈**（见文件头那一节）。
    pub const ROAD_MAX: usize = 8;

    /// 立一棵树：注入的两枚戳子与那一次动作跟着树走——它们对每一条同值，故不必逐个作参数传。
    ///
    /// `stamps` 收成**一格具名的组**（照实记：两枚戳子是同型的函数指针，摆成位置参数时写反了
    /// 编不过才算数——`session` 那一族认 `owner` 还是 `grantor` 实测栽过一次）。
    pub const fn new(stamps: Stamps, unship: Unship) -> Operator {
        Operator {
            root: Vec::new(),
            slots: Vec::new(),
            stamps,
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
    /// - 是一枚 `Tile`：**先探一次**（[`VestedBy`]）——答不出 ⇒ 当场剔掉那一格、放下那一份
    ///   （[`Unship`]），答 [`Fail::Dead`]；答得出 ⇒ 交给 `ship`。
    ///
    /// `ship` 是"交出去"那一手（适配层在这里把 Pie 授给调用方，核心因此不碰内核）——与同文件
    /// 另一个注入事实 [`Unship`] 正好是一式的两半（交出 / 放下）。
    pub fn find(&mut self, id: EntryId, mut ship: impl FnMut(PieToken)) -> Result<(), Fail> {
        let vested_by = self.stamps.vested_by;
        let unship = self.unship;
        // 先只读地问一遍（借用到此为止），再决定要不要动树。
        let pie = match self.slot(id) {
            None => return Err(Fail::Unknown),
            Some(slot) => match &slot.node {
                Node::Pane(_) => return Err(Fail::NotATile),
                Node::Tile(pie) => *pie,
            },
        };
        if vested_by(pie).is_none() {
            let _ = self.unlink(id);
            let _ = unship(pie);
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
    /// 照实记：`Dead` 那一格是**三因一码**（不是孔 / 不在我表里 / 已封印），与 [`VestedBy`] 的
    /// 口径同一句。**不许拆它**——今天没有一位客人分得开这三因，分开就是造三个没人读的格。
    ///
    /// 谁问它：持树者判 [`Rule::Opens`](super::judge::Rule::Opens) 时，把这一格翻成开者，再拿
    /// 开者去问名册"这一刻代表谁"。故它答的是 **TID**，不是身份号——两件事两个落点。
    pub fn opens(&self, id: EntryId) -> Result<TaskId, Fail> {
        let opened_by = self.stamps.opened_by;
        let pie = match self.slot(id) {
            None => return Err(Fail::Unknown),
            Some(slot) => match &slot.node {
                Node::Pane(_) => return Err(Fail::NotATile),
                Node::Tile(pie) => *pie,
            },
        };
        opened_by(pie).ok_or(Fail::Dead)
    }

    /// **剪**：把那一号那一格剪掉。
    ///
    /// 那一格得存在（否则 [`Fail::Unknown`]）；是 `Pane` 的话**必须空着**（否则 [`Fail::NonEmpty`]）。
    /// 剪掉一枚 `Tile` 时那一枚放下（[`Unship`]）——它是资源实体的一份引用，不放下就漏水。
    ///
    /// **剪掉的那一槽留成墓碑**（`None`），不 `remove`：号是下标，一移后面全错位。故一枚剪过的
    /// 号从此答 [`Fail::Unknown`]，而**它不会被重新铸出来**（水位只增）。
    pub fn trim(&mut self, id: EntryId) -> Result<(), Fail> {
        let unship = self.unship;
        let dropped = match self.slot(id) {
            None => return Err(Fail::Unknown),
            Some(slot) => match &slot.node {
                Node::Pane(inner) if inner.is_empty() => None,
                Node::Pane(_) => return Err(Fail::NonEmpty),
                Node::Tile(pie) => Some(*pie),
            },
        };
        let _ = self.unlink(id);
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
        Ok(self.kids(at)?.iter().copied())
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
        let mut level: &[EntryId] = &self.root;
        for step in rest {
            let child = self.child(level, *step).ok_or(Fail::Unknown)?;
            level = match &self.slot(child).ok_or(Fail::Unknown)?.node {
                Node::Pane(inner) => inner,
                Node::Tile(_) => return Err(Fail::NotAPane),
            };
        }
        self.child(level, *last).ok_or(Fail::Unknown)
    }

    /// **名**：这枚号此刻叫什么。
    ///
    /// **一趟查表**（照实记：原来是**一趟全树扫**——那时号不是下标；这一版里
    /// [`Operator::slot`] 直接取，故 `name` 与 `find` / `trim` 一样是 O(1)）。
    ///
    /// 号失效（`trim` 剪掉、[`Operator::find`] 剔死）与从来没铸过**长得一样** ⇒ 都答
    /// [`Fail::Unknown`]：表里那一槽是个墓碑，而墓碑不对外说话。
    pub fn name(&self, id: EntryId) -> Result<Name, Fail> {
        self.slot(id).map(|slot| slot.name).ok_or(Fail::Unknown)
    }

    // ── 走路 ────────────────────────────────────────────────

    /// 落 / 分共用的那一手：在 `at` 那一块 `Pane` 里给 `name` 立一格（或换绑那一格）。
    ///
    /// **答那一格自己的号**：换绑不动号，故只有"真铸了一格"才动水位。**不递归**——
    /// `at` 那一块 `Pane` 就是一次 [`Operator::kids`]。
    ///
    /// **先只读地问一遍**（那一格叫什么号），再动手：这样动手那一段只需要一次
    /// `slots[i]` 的可变借用，不必在 children 里穿一层 `&mut`（那正是老一版递归的由头）。
    fn put(&mut self, at: Where, name: Name, node: Node, want: Want) -> Result<EntryId, Fail> {
        let unship = self.unship;
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
                    Node::Tile(old) => Some(*old),
                    Node::Pane(inner) if inner.is_empty() => None,
                    // 落到这里只可能是 `Want::Tile`：换绑会毁掉那块非空 `Pane` 里的东西。
                    Node::Pane(_) => return Err(Fail::NonEmpty),
                };
                slot.node = node;
                if let Some(old) = old {
                    let _ = unship(old);
                }
                Ok(id)
            }
            None => {
                if self.kids(at)?.len() >= Self::PANE_CAP {
                    return Err(Fail::Full);
                }
                // **先要位、再落格**：条数那一闸管的是`PANE_CAP`，这两行管**内存**。
                // 少了它们，分配失败走的是 `handle_alloc_error`（abort）——而同一句"备不下就
                // 如实报"在仓里另外两处都是 `try_reserve → Full`：`Desk::admit`
                // （`programs/src/system/operator/desk.rs`）与 `Ledger::grow`
                // （`crates/contract/src/system/operator/ledger.rs`）。**同一句话，三处一个纪律。**
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
                    Node::Tile(_) => Err(Fail::NotAPane),
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
                    Node::Tile(_) => Err(Fail::NotAPane),
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

    /// 在这一块 `Pane` 的孩子里按名字找那个号（只读一趟扫，最多 `PANE_CAP` 次查表）。
    fn child(&self, level: &[EntryId], name: Name) -> Option<EntryId> {
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
