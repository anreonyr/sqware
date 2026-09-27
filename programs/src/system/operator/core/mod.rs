//! operator::core — **树那一本账**：一张按号排的表 ＋ 八条原语。
//!
//! **照实记（它原先住 `protocol::system::operator::core`）**：那一份的读者只有本域的持树者
//! （`prog-operator` 那一枚线程）——按"协议 = 共享语言"的判据，它属于实现侧。协议那一侧
//! 只留**上线的类型**（`EntryId` / `Where` / `Fail` / `Id` / `Rule` / `Ruling` 与两条容量）
//! 与客侧几手。
//!
//! 本目录四份：`mod.rs` 账 ＋ `ledger.rs` 归属与规矩 ＋ `judge.rs` 门外那一问 ＋
//! `gate.rs` 裁决折成线上一格。**判定与账同住这一侧**，故 `gate` 那三格线上码与
//! `frame` 的同步断言也搬到这里（见本文件末尾）。

use alloc::vec::Vec;

use env::{Name, PieToken, TaskId};

use protocol::communication::establish::{opened_by, vested_by};
use protocol::system::operator::frame::{PANE_CAP, ROAD_MAX};
use protocol::system::operator::{EntryId, Fail, Where};

// ── 三个子模块 ──────────────────────────────────────────────
pub mod gate;
pub mod judge;
pub mod ledger;

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
    /// 四条判据，一条不多：
    ///
    /// - `at` 那块 `Pane` 得在（号不在 ⇒ [`Fail::Unknown`]）；它是一枚 `Tile` ⇒ [`Fail::NotAPane`]；
    /// - `name` 那一格空着 ⇒ **铸一枚新号**放上去；
    /// - `name` 那一格已占 ⇒ **换绑**：旧的那一枚放下（`mail::release`）、**号不动**，答原来那一枚号
    ///   ——除非它是一块**非空** `Pane`（⇒ [`Fail::NonEmpty`]：要动它先清空）；
    /// - 那一块 `Pane` 已经有 [`PANE_CAP`] 条 ⇒ [`Fail::Full`]。
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
    /// - 那一块 `Pane` 已经有 [`PANE_CAP`] 条 ⇒ [`Fail::Full`]。
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
                Node::Tile(pie) => *pie,
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
    /// 谁问它：持树者判 [`Rule::Opens`](judge::Rule::Opens) 时，把这一格翻成开者，再拿
    /// 开者去问名册"这一刻代表谁"。故它答的是 **TID**，不是身份号——两件事两个落点。
    pub fn opens(&self, id: EntryId) -> Result<TaskId, Fail> {
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
                Node::Tile(pie) => Some(*pie),
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
    /// [`Fail::NotAPane`]；路超过 [`ROAD_MAX`] 段 ⇒ [`Fail::Full`]。
    /// 走到头答**那一格自己的号**——故**最后一段是一枚 `Tile` 也行**（那正是门牌那一格：
    /// `/device/uart` 到头就是一枚砖）。
    ///
    /// **空路 ⇒ [`Fail::Unknown`]**（对照 [`Operator::list`]：它空路却能列——列的是根那一层，
    /// 不需要根有号）：**根没有号**，没什么可译。
    ///
    /// 与 `list` 一样**不过问死活**：剔死是 [`Operator::find`] 那一路上的事。
    pub fn seek(&self, road: &[Name]) -> Result<EntryId, Fail> {
        if road.len() > ROAD_MAX {
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
                // 如实报"在仓里另外两处都是 `try_reserve → Full`：`Desk::admit`
                // （`crates/protocol/src/system/desk.rs`）与 `Ledger::land`
                // （`crates/protocol/src/system/operator/core/ledger.rs`）。**同一句话，三处一个纪律。**
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

// ── 同步义务：`gate` 那三格线上码与 `frame` 的对照表 ──────────────
//
// 真正的对照表只有一份（`protocol::system::operator::frame`）；`gate` 为了"不带载体"自己
// 拿了一份，故在这里**编译期**把两者钉住——一漂就编不过。**这一条住这里**：只有这一层同时
// 看得见 `gate` 与 `frame`。
const _: () = {
    use protocol::system::operator::frame;
    assert!(gate::WIRE_OK == frame::OK);
    assert!(gate::WIRE_DENIED == frame::DENIED);
    assert!(gate::WIRE_UNJUDGED == frame::UNJUDGED);
};
