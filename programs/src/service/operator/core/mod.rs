//! 一张按号排的表 ＋ 七条线上原语 ＋ 三条给判据的。

use alloc::string::String;
use alloc::vec::Vec;

use env::{PieToken, TaskId};

use protocol::common::path::Path;
use protocol::communication::establish::{opened_by, vested_by};
use protocol::service::operator::frame::PANE_CAP;
use protocol::service::operator::{EntryId, Fail, Permit, Where};

pub mod gate;
pub mod judge;

/// **一格**：名字 + 去处。它住在 Operator::slots 里，**下标就是它的号**。
/// 名字只是一段（String：不长于 255 字节那一格由**编帧**那一刻判），不是整条路——路那一层只剩
/// Operator::seek 在用。
struct Slot {
    name: String,
    node: Node,
}

/// 去处：一块 Node::Pane（窗格，还能往里走）或一枚 Node::Tile（砖，到头了）。
enum Node {
    /// 一块 Pane（窗格）：里面是**孩子的号**（按登记序）。**还能往里走**。
    Pane(Vec<EntryId>),
    /// 一枚 Tile（砖）：到头了，就是内核给的那一枚句柄；**「用」与「改」两轴都住在它上面**。
    /// **两轴为什么不挂 `Slot`**：① 许可只在 `find` 那一问上被读，而 `find` 只认砖 ⇒ 它本来就
    /// 是"一枚砖的性质"；② 挂 `Slot` 会造出"一块 `Pane` 也有许可 / 也有主人"这两格，而它们
    Tile {
        pie: PieToken,
        permit: Permit,
        owner: Option<TaskId>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Want {
    /// 要一枚 `Tile`（Operator::land）。
    Tile,
    /// 要一块 `Pane`（Operator::part）。
    Pane,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Key {
    /// 坐标：那一块 `Pane` + 那一段名字（`land` / `part` 那一问手里有的）。
    At(Where, String),
    /// 号：条目自己的号（`find` / `trim` 手里有的）。
    Id(EntryId),
}

/// 一棵命名树：**一个 Operator 管着所有条目**。
/// 根 = `root` 那一叠**孩子的号**（Where::Root 指的就是它）；其余每一格住 `slots` 里，
/// **号就是下标**。两个容器的容量**互不牵连**：`root` 管根那一层，`slots` 管"一共铸过几格"。
pub struct Operator {
    root: Vec<EntryId>,
    /// 一叠格子：**只增不减**（`unlink` 只把那一格置空，不 `pop`）。
    /// ⇒ **下标即号、号不重用**：一枚旧号要么还指着原来那一格，要么指着墓碑
    /// （Fail::Unknown），不会悄悄指到后铸的那一格身上。铸号因此**不必另立水位**——水位
    slots: Vec<Option<Slot>>,
}

impl Operator {
    pub const fn new() -> Operator {
        Operator {
            root: Vec::new(),
            slots: Vec::new(),
        }
    }

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

    pub fn part(&mut self, at: Where, name: String) -> Result<EntryId, Fail> {
        self.put(at, name, Node::Pane(Vec::new()), Want::Pane)
    }

    /// **寻**：把那一号后面那一枚 Pie 交出去。
    /// - 号不在树上 ⇒ Fail::Unknown（剪掉、剔死、从没铸过长得一样）；
    /// - 那一格是一块 `Pane` ⇒ Fail::NotATile；
    /// - 是一枚 `Tile`：**先探一次**（vested_by）——答不出 ⇒ 当场剔掉那一格、放下那一份
    ///   （mail::release），答 Fail::Dead；答得出 ⇒ 交给 `ship`。
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
    /// 与 Operator::find **正相反**：这一条什么都不交出去——只答"是谁"，不把那枚句柄递出去；
    /// 三格，与 `find` 对同一格答**同一批码**（差别只在"活着的砖"那一档：`find` 交出句柄，
    /// - 号不在树上（墓碑 / 从没铸过）⇒ Fail::Unknown；
    /// - 那一格是一块 `Pane` ⇒ Fail::NotATile（**没有开者这一说**）；
    /// - 是一枚 `Tile`，但开者那扇门封印了 ⇒ Fail::Dead。
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
    /// 那一格得存在（否则 Fail::Unknown）；是 `Pane` 的话**必须空着**（否则 Fail::NonEmpty）。
    /// 剪掉一枚 `Tile` 时那一枚放下（mail::release）——它是资源实体的一份引用，不放下就漏水。
    /// **剪掉的那一槽留成墓碑**（`None`），不 `remove`：号是下标，一移后面全错位。故一枚剪过的
    /// 号从此答 Fail::Unknown，而**它不会被重新铸出来**（水位只增）。
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

    /// **列**：看那一块 `Pane` 里有哪些**号**（Where::Root = 根那一层）。
    /// 号不在 ⇒ Fail::Unknown；那一号是一枚 `Tile` ⇒ Fail::NotAPane。**不过问死活**：
    /// 剔死是 Operator::find 那一路上的事。
    /// 答的是号、不是名字（机器用号，人用名——名字另问 Operator::name）。
    /// 顺序是**号序**：pane 里本来是登记序，而号单调 ⇒ 两者一致，不必额外排。
    pub fn list(&self, at: Where) -> Result<impl Iterator<Item = EntryId> + '_, Fail> {
        Ok(self.kids(at)?.iter().copied())
    }

    pub fn seek(&self, road: &Path) -> Result<EntryId, Fail> {
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
    pub fn name(&self, id: EntryId) -> Result<String, Fail> {
        self.slot(id)
            .map(|slot| slot.name.clone())
            .ok_or(Fail::Unknown)
    }

    pub fn permit(&self, id: EntryId) -> Permit {
        match self.slot(id) {
            Some(Slot {
                node: Node::Tile { permit, .. },
                ..
            }) => *permit,
            _ => Permit::Unset,
        }
    }

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

    /// 落 / 分共用的那一手：在 `at` 那一块 `Pane` 里给 `name` 立一格（或换绑那一格）。
    /// **答那一格自己的号**：换绑不动号，故只有"真铸了一格"才动水位。**不递归**——
    /// `at` 那一块 `Pane` 就是一次 Operator::kids。
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
                // **分那边幂等**：想要的就是"这儿是一块 `Pane`"，已经在就是成了（Want::Pane）。
                if want == Want::Pane && matches!(slot.node, Node::Pane(_)) {
                    return Ok(id);
                }
                let old = match &slot.node {
                    Node::Tile { pie: old, .. } => Some(*old),
                    Node::Pane(inner) if inner.is_empty() => None,
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
                // 如实报"在仓里另外一处是 `try_reserve → Full`：Desk::admit
                // （`programs/src/system/common/face/desk.rs`）。**同一句话，两处一个纪律**
                self.slots.try_reserve(1).map_err(|_| Fail::Full)?;
                self.kids_mut(at)?.try_reserve(1).map_err(|_| Fail::Full)?;
                let fresh = EntryId::new(self.slots.len());
                self.slots.push(Some(Slot { name, node }));
                self.kids_mut(at)?.push(fresh);
                Ok(fresh)
            }
        }
    }

    /// 这一块 `Pane` 的孩子（Where::Root = 根那一层；`At(id)` = 那一格，得是 `Pane`）。
    /// 三格判据与老一版的走法逐条对齐：号不在 ⇒ Fail::Unknown；那一格是 `Tile` ⇒
    /// Fail::NotAPane。
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

    /// Operator::kids 的可变那一半。
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
    fn slot(&self, id: EntryId) -> Option<&Slot> {
        self.slots.get(id.get()).and_then(Option::as_ref)
    }

    /// 按 Key 那两把钥匙取那一格（只读）。两种寻址**打的是同一格**（见 Key）。
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

// 真正的对照表只有一份（protocol::service::operator::frame）；`gate` 为了"不带载体"自己
// 看得见 `gate` 与 `frame`。
const _: () = {
    use protocol::service::operator::frame;
    assert!(gate::WIRE_OK == frame::OK);
    assert!(gate::WIRE_DENIED == frame::DENIED);
    assert!(gate::WIRE_UNJUDGED == frame::UNJUDGED);
};
