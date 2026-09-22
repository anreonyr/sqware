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

use alloc::vec::Vec;

use env::{Name, PieToken, TaskId};

// ── 结构 ────────────────────────────────────────────────────

/// 一枚条目的**号**：机器用的那一个。
///
/// **裸号**：与 [`PolicyId`](crate::principal::PolicyId) / [`CoalitionId`](crate::coalition::CoalitionId)
/// 同形（8 字节小端上线），不同源。线上解码面造得出任何号（[`EntryId::new`]），
/// "这枚号还在不在"由每条读查一次树答出来。
///
/// **没有 `ROOT`**（对照另两种号：那两处的 `ROOT` 都在，这里特意没有）：根不是谁条目里的
/// 一条，故**根没有号**——`EntryId(0)` 是第一个**真格子**（`sys`），不是"没有"。
/// "没有这个号"由 [`Fail::Unknown`] 答，别拿 0 当空。
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
/// 名字只是**一段**（`Name`：定长 32 字节、构造即校验），不是整条路。
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

/// 六条原语会失败在哪一格。**一格对应一个不同的下一步**。
///
/// **没有"名字已被占"那一格**：同名接手一枚 `Tile`、或一块**空的** `Pane`，都是换绑
/// （见 [`Operator::land`] / [`Operator::part`]）；而 owner 归 Principal，Operator 分不出
/// "自己 / 别人"，所以"已占即拒"在这里无处落脚。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    /// 路上没有这一段 ⇒ 换个名字，或者先把中间层分出来。
    ///
    /// **空路也走这一格**：空路是"根"本身，而根不是谁条目里的一条——落 / 分 / 剪对根都动手不了，
    /// 译也对根使不上（**根没有号**，没什么可译）。
    Unknown,
    /// 那块 `Pane` 里还有东西 ⇒ 先清空。
    NonEmpty,
    /// 寻到头是一块 `Pane`，不是一枚 `Tile` ⇒ 改用列，或者再往下走一段。
    NotATile,
    /// 那一段不是一块 `Pane`（是一枚 `Tile`）⇒ 走不过去；列的时候则说明"那是枚 `Tile`，没什么可列"。
    NotAPane,
    /// 一块 `Pane` 装不下，或者一条路太长 ⇒ 拆层 / 扩容量。
    Full,
    /// 那枚 Pie 后面的人没了（探不到）⇒ 重落 / 重寻。**剔掉那一条的同时**答这一格。
    Dead,
}

/// **活性**：那一枚 Pie 还答得出吗？答不出（`None`）= 它后面的人没了。
///
/// 与 `system::board` 同一格（`VestedBy` 的形状照旧）：本正文没有 owner，故这里只取"答得出吗"，
/// 答出来的 `TaskId` 用不到。
pub type VestedBy = fn(PieToken) -> Option<TaskId>;

/// **放下**：把我这一份自释。剪掉或换掉一枚 `Tile` 时用它——不加这一格，那一枚句柄就漏在树里。
pub type Unship = fn(PieToken) -> Result<(), ()>;

// ── 树 ──────────────────────────────────────────────────────

/// 一棵命名树：**一个 Operator 管着所有条目**。
///
/// 根 = `root` 那一叠条目；**空路就是根**（[`Operator::list`] 列的就是它那一层）。
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
    /// 一条路最多几段。深也是策略。
    pub const PATH_MAX: usize = 8;

    /// 立一棵树：两个注入的机制事实跟着树走——它们对每一条同值，故不必逐个作参数传。
    pub const fn new(vested_by: VestedBy, unship: Unship) -> Operator {
        Operator {
            root: Vec::new(),
            next: 0,
            vested_by,
            unship,
        }
    }

    /// **落**：把一枚 Pie 落到一条路上（放一枚 `Tile`）。
    ///
    /// 四条判据，一条不多：
    ///
    /// - 路非空、有界（空路 ⇒ [`Fail::Unknown`]；超深 ⇒ [`Fail::Full`]）；
    /// - 路上除最后一段外**都得是 `Pane` 且存在**（缺一段 ⇒ [`Fail::Unknown`]；
    ///   那一段是一枚 `Tile` ⇒ [`Fail::NotAPane`]）；
    /// - 最后一段空着 ⇒ 落上；
    /// - 最后一段已经占着 ⇒ **换绑**：旧的那一枚放下（[`Unship`]）——除非它是一块**非空** `Pane`
    ///   （⇒ [`Fail::NonEmpty`]：要动它先清空）。
    pub fn land(&mut self, path: &[Name], pie: PieToken) -> Result<(), Fail> {
        Self::checked(path)?;
        let unship = self.unship;
        let fresh = EntryId::new(self.next);
        let last = path[path.len() - 1];
        let level = self.level_mut(path)?;
        match level.iter().position(|e| e.name == last) {
            Some(at) => {
                // 已占：只有"一枚 `Tile`"与"空 `Pane`"可以换绑；前者那一枚要放下。
                // **换绑不动号**：那一格还是同一格。

                let old = match &level[at].node {
                    Node::Tile(old) => Some(*old),
                    Node::Pane(inner) if inner.is_empty() => None,
                    Node::Pane(_) => return Err(Fail::NonEmpty),
                };
                level[at].node = Node::Tile(pie);
                if let Some(old) = old {
                    let _ = unship(old);
                }
                Ok(())
            }
            None => {
                if level.len() >= Self::PANE_CAP {
                    return Err(Fail::Full);
                }
                level.push(Entry {
                    id: fresh,
                    name: last,
                    node: Node::Tile(pie),
                });
                self.next += 1;
                Ok(())
            }
        }
    }

    /// **分**：在一条路上分出一块**空的** `Pane`（开一块窗格）。
    ///
    /// 门槛与 [`Operator::land`] 同：路非空、有界，中间那几层都得是 `Pane` 且存在。
    /// 最后一段：空着 ⇒ 放一块空 `Pane`；已是 `Tile` ⇒ 换掉它（旧的那一枚放下）；
    /// 已是**空** `Pane` ⇒ 无事；已是**非空** `Pane` ⇒ [`Fail::NonEmpty`]。
    pub fn part(&mut self, path: &[Name]) -> Result<(), Fail> {
        Self::checked(path)?;
        let unship = self.unship;
        let fresh = EntryId::new(self.next);
        let last = path[path.len() - 1];
        let level = self.level_mut(path)?;
        match level.iter().position(|e| e.name == last) {
            Some(at) => {
                let old = match &level[at].node {
                    Node::Pane(inner) if inner.is_empty() => return Ok(()),
                    Node::Pane(_) => return Err(Fail::NonEmpty),
                    Node::Tile(old) => Some(*old),
                };
                level[at].node = Node::Pane(Vec::new());
                if let Some(old) = old {
                    let _ = unship(old);
                }
                Ok(())
            }
            None => {
                if level.len() >= Self::PANE_CAP {
                    return Err(Fail::Full);
                }
                level.push(Entry {
                    id: fresh,
                    name: last,
                    node: Node::Pane(Vec::new()),
                });
                self.next += 1;
                Ok(())
            }
        }
    }

    /// **寻**：走到头，把那一枚 Pie 交出去。
    ///
    /// - 走不动（中间那一段是一枚 `Tile`）⇒ [`Fail::NotAPane`]；缺一段 ⇒ [`Fail::Unknown`]；
    /// - 到头是一块 `Pane` ⇒ [`Fail::NotATile`]（**空路也是这一格**：根是一块 `Pane`）；
    /// - 到头是一枚 `Tile`：**先探一次**（[`VestedBy`]）——答不出 ⇒ 当场剔掉那一条、放下那一份
    ///   （[`Unship`]），答 [`Fail::Dead`]；答得出 ⇒ 交给 `give`。
    ///
    /// `give` 是"交出去"那一手（适配层在这里把 Pie 授给调用方，核心因此不碰内核）。
    pub fn find(&mut self, path: &[Name], mut give: impl FnMut(PieToken)) -> Result<(), Fail> {
        if path.len() > Self::PATH_MAX {
            return Err(Fail::Full);
        }
        if path.is_empty() {
            return Err(Fail::NotATile);
        }
        let vested_by = self.vested_by;
        let unship = self.unship;
        let last = path[path.len() - 1];
        let level = self.level_mut(path)?;
        let at = level
            .iter()
            .position(|e| e.name == last)
            .ok_or(Fail::Unknown)?;
        let pie = match &level[at].node {
            Node::Pane(_) => return Err(Fail::NotATile),
            Node::Tile(pie) => *pie,
        };
        if vested_by(pie).is_none() {
            level.remove(at);
            let _ = unship(pie);
            return Err(Fail::Dead);
        }
        give(pie);
        Ok(())
    }

    /// **剪**：把路上那一条剪掉。
    ///
    /// 那一条得存在；是 `Pane` 的话**必须空着**（否则 [`Fail::NonEmpty`]）。
    /// 剪掉一枚 `Tile` 时那一枚放下（[`Unship`]）——它是资源实体的一份引用，不放下就漏水。
    pub fn trim(&mut self, path: &[Name]) -> Result<(), Fail> {
        Self::checked(path)?;
        let unship = self.unship;
        let last = path[path.len() - 1];
        let level = self.level_mut(path)?;
        let at = level
            .iter()
            .position(|e| e.name == last)
            .ok_or(Fail::Unknown)?;
        let dropped = match &level[at].node {
            Node::Pane(inner) if inner.is_empty() => None,
            Node::Pane(_) => return Err(Fail::NonEmpty),
            Node::Tile(pie) => Some(*pie),
        };
        level.remove(at);
        if let Some(pie) = dropped {
            let _ = unship(pie);
        }
        Ok(())
    }

    /// **列**：看一块 `Pane` 里有哪些**号**。
    ///
    /// **空路 = 根**（列根那一层）。缺一段 ⇒ [`Fail::Unknown`]；走不动或到头是一枚 `Tile`
    /// ⇒ [`Fail::NotAPane`]。**不过问死活**：剔死是 [`Operator::find`] 那一路上的事。
    ///
    /// 答的是号、不是名字（机器用号，人用名——名字另问 [`Operator::name`]）。
    /// 顺序是**号序**：pane 里本来是登记序，而号单调 ⇒ 两者一致，不必额外排。
    pub fn list(&self, path: &[Name]) -> Result<impl Iterator<Item = EntryId> + '_, Fail> {
        if path.len() > Self::PATH_MAX {
            return Err(Fail::Full);
        }
        let mut level = &self.root;
        for step in path {
            let entry = level
                .iter()
                .find(|e| e.name == *step)
                .ok_or(Fail::Unknown)?;
            level = match &entry.node {
                Node::Pane(inner) => inner,
                Node::Tile(_) => return Err(Fail::NotAPane),
            };
        }
        Ok(level.iter().map(|e| e.id))
    }

    /// **译**：把一条路**译成那一枚号**——名字只能走到这一格，往下一律按号。
    ///
    /// 走法与 [`Operator::list`] 一模一样（缺一段 ⇒ [`Fail::Unknown`]，中途是一枚 `Tile`
    /// ⇒ [`Fail::NotAPane`]，超深 ⇒ [`Fail::Full`]）；差别只在最后一步：`list` 答那一块
    /// `Pane` 里的号，`seek` 答**走到的那一格自己的号**——故**最后一段是一枚 `Tile` 也行**
    /// （那正是门牌那一格：`/device/uart` 到头就是一枚砖）。
    ///
    /// **空路 ⇒ [`Fail::Unknown`]**（对照 [`Operator::list`]：它空路却能列——列的是根那一层，
    /// 不需要根有号）：**根没有号**，没什么可译。
    ///
    /// 与 `list` 一样**不过问死活**：剔死是 [`Operator::find`] 那一路上的事。
    pub fn seek(&self, road: &[Name]) -> Result<EntryId, Fail> {
        if road.len() > Self::PATH_MAX {
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
    /// 删格要维护，本刀不取。
    ///
    /// 号失效（`trim` 剪掉、[`Operator::find`] 剔死）与从来没铸过**长得一样** ⇒ 都答
    /// [`Fail::Unknown`]：水位只往上走，本层不记墓碑，也分不出这两件事。
    pub fn name(&self, id: EntryId) -> Result<Name, Fail> {
        fn seek(level: &[Entry], id: EntryId) -> Option<Name> {
            for entry in level {
                if entry.id == id {
                    return Some(entry.name);
                }
                if let Node::Pane(inner) = &entry.node {
                    if let Some(found) = seek(inner, id) {
                        return Some(found);
                    }
                }
            }
            None
        }
        seek(&self.root, id).ok_or(Fail::Unknown)
    }

    // ── 走路 ────────────────────────────────────────────────

    /// 落 / 分 / 剪共同的两条门槛：**路非空**（空路是根本身）、**路不超深**。
    fn checked(path: &[Name]) -> Result<(), Fail> {
        if path.is_empty() {
            return Err(Fail::Unknown);
        }
        if path.len() > Self::PATH_MAX {
            return Err(Fail::Full);
        }
        Ok(())
    }

    /// 走到 `path` 的**上一层**（最后一段所在的那一叠）。
    ///
    /// **调用方先过 [`Operator::checked`]**（或自己挡住空路）：这里按 `path[..len - 1]` 切，
    /// 空路上切不出来。
    fn level_mut(&mut self, path: &[Name]) -> Result<&mut Vec<Entry>, Fail> {
        let mut level = &mut self.root;
        for step in &path[..path.len() - 1] {
            let at = level
                .iter()
                .position(|e| e.name == *step)
                .ok_or(Fail::Unknown)?;
            level = match &mut level[at].node {
                Node::Pane(inner) => inner,
                Node::Tile(_) => return Err(Fail::NotAPane),
            };
        }
        Ok(level)
    }
}
