//! 一张按号排的表 ＋ 八条线上原语 ＋ 三条给判据的。

use alloc::string::String;
use alloc::vec::Vec;

use env::{PieToken, TaskId};

use protocol::common::path::{Path, PathBuf};
use protocol::communication::session::{alive, opened_by};
use protocol::system::operator::{PANE_CAP, Kind, EntryId, Fail, Permit, Where};
use env::pie;
use runtime::core::res::pie::{pies};

pub mod gate;
pub mod judge;

/// **一格**：名字 + 去处。它住在 Operator::slots 里，**下标就是它的号**
/// 名字只是一段（String：不长于 255 字节那一格由**编帧**那一刻判），不是整条路——路那一层只剩
struct Slot {
    name: String,
    node: Node,
}

/// 去处：一块 Node::Pane（窗格，还能往里走）或一枚 Node::Tile（砖，到头了）
enum Node {
    /// 一块 Pane（窗格）：里面是**孩子的号**（按登记序）。**还能往里走**
    Pane(Vec<EntryId>),
    /// 一枚 Tile（砖）：到头了，就是内核给的那一枚句柄；**「用」与「改」两轴都住在它上面**
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
    /// 要一枚 `Tile`（Operator::land）
    Tile,
    /// 要一块 `Pane`（Operator::part）
    Pane,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Key<'a> {
    /// 坐标：那一块 `Pane` + 那一段名字（`land` / `part` 那一问手里有的）
    At(Where, &'a str),
    /// 号：条目自己的号（`find` / `trim` 手里有的）
    Id(EntryId),
}

/// **一次改动**：树真的变了才有它（那一格自己的号 ＋ 它是哪一种 ＋ 归属）。
///
/// **不含"路"那一格**：路能从号现走（[`Operator::road_to`]），而 `trim` 正是"号还在、格子没了"
/// ——那一刻才是唯一需要**先**把路记下来的地方。故这里只记那三格，路在落那条事件时现走。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Change {
    /// 哪一种：新铸 / 换绑 / 新立一块 / 剪掉
    pub kind: Kind,
    /// 那一格自己的号
    pub id: EntryId,
    /// 那一格的主人（无主 = 零号）
    pub owner: TaskId,
    /// **剪掉那一档才填**：剪完槽就空了，路只能**剪之前**记下来（其余三档由 `publish` 现走）。
    pub road: Option<PathBuf>,
}

/// 一棵命名树：**一个 Operator 管着所有条目**
/// 根 = `root` 那一叠**孩子的号**（Where::Root 指的就是它）；其余每一格住 `slots` 里
/// **号就是下标**。两个容器的容量**互不牵连**：`root` 管根那一层，`slots` 管"一共铸过几格"
pub struct Operator {
    root: Vec<EntryId>,
    /// 一叠格子：**只增不减**（`unlink` 只把那一格置空，不 `pop`）
    /// ⇒ **下标即号、号不重用**：一枚旧号要么还指着原来那一格，要么指着墓碑
    /// （Fail::Unknown），不会悄悄指到后铸的那一格身上。铸号因此**不必另立水位**——水位
    slots: Vec<Option<Slot>>,
}

pub struct Location {
    pub at: Where,
    pub name: String,
}
#[derive(Clone, Copy)]
pub struct Tile {
    pub pie: PieToken,
    pub permit: Permit,
    pub owner: Option<TaskId>,
}

impl Operator {
    pub const fn new() -> Operator {
        Operator {
            root: Vec::new(),
            slots: Vec::new(),
        }
    }

    /// **落**：`at` 那一块里给 `name` 贴一枚 `Tile`，并答**这一次真动了什么**。
    ///
    /// 新铸那一档是 [`Kind::Landed`]，换绑那一档是 [`Kind::Rebound`]（**号不动**）——
    /// 树一个字节没变的那一档不存在：`put` 只在"造了一格"或"换掉了那一格的两轴"两种下场里返 `Ok`。
    pub fn land(&mut self, location: Location, tile: Tile) -> Result<Change, Fail> {
        let Location { at, name } = location;
        let Tile { pie, permit, owner } = tile;
        // **归属原样带过去**：`None` = 无主（谁都能接手），`Some(w)` = 有主。**不许折成零号**——
        // `claimable` 读的就是这一格（零号是一个"永远不在场"的主人，与"无主"不是一件事）。
        let who = owner.unwrap_or(TaskId::new(0));
        // **换绑还是新铸**：落之前先只读地问一遍（那一手与 `put` 开头那一趟是同一件事）。
        let was_tile = self.kid(at, name.as_str())?.is_some();
        let id = self.put(Location { at, name }, Node::Tile { pie, permit, owner })?;
        Ok(Change {
            kind: if was_tile {
                Kind::Rebound
            } else {
                Kind::Landed
            },
            id,
            owner: who,
            road: None,
        })
    }

    /// **分**：`at` 那一块里立一块空窗格（幂等：已经在就是成了）。返那一格自己的号。
    pub fn part(&mut self, at: Where, name: String) -> Result<EntryId, Fail> {
        Ok(self.part_at(at, name)?.0)
    }

    /// **分 ＋ 真动了什么**：**真造了一块**才记 [`Kind::Parted`]（已是窗格那一档 `changed = false`
    /// ——树一个字节没变，不该报一条事件）。**两档都答那一格自己的号**（幂等那一档要把它查出来）。
    pub fn part_at(
        &mut self,
        at: Where,
        name: String,
    ) -> Result<(EntryId, bool, Option<Change>), Fail> {
        if let Some(id) = self.kid(at, name.as_str())? {
            return Ok((id, false, None));
        }
        let id = self.put(Location { at, name }, Node::Pane(Vec::new()))?;
        Ok((
            id,
            true,
            Some(Change {
                kind: Kind::Parted,
                id,
                owner: TaskId::new(0),
                road: None,
            }),
        ))
    }

    /// `at` 那一块里叫 `name` 的那一格（没有 ⇒ `None`）。`put` 那一趟"先只读地问一遍"
    /// 就是它——故这一只手是那件事的公开发法，不是另一份实现。
    pub fn kid(&self, at: Where, name: &str) -> Result<Option<EntryId>, Fail> {
        Ok(self
            .kids(at)?
            .iter()
            .copied()
            .find(|child| self.slot(*child).is_some_and(|slot| slot.name == name)))
    }

    /// **寻**：把那一号后面那一枚 Pie 交出去
    /// - 号不在树上 ⇒ Fail::Unknown（剪掉、剔死、从没铸过长得一样）
    /// - 那一格是一块 `Pane` ⇒ Fail::NotATile
    /// - 是一枚 `Tile`：**先问一句存活**（`establish::alive`）——不在 ⇒ 当场剔掉那一格、
    /// 放下那一份（pie::release），答 Fail::Dead；在 ⇒ 交给 `ship`
    /// `ship` 是"交出去"那一手（**回调**：适配层把那一枚授给调用方）——与"放下"正好是
    /// 一式的两半（交出 / 放下）。它留成参数而不直接叫，是因为它带**去授给谁**那一格
    /// （调用方手里那个号），不是无参动作
    pub fn find(&mut self, id: EntryId, mut ship: impl FnMut(PieToken)) -> Result<(), Fail> {
        // 先只读地问一遍（借用到此为止），再决定要不要动树。
        let pie = match self.slot(id) {
            None => return Err(Fail::Unknown),
            Some(slot) => match &slot.node {
                Node::Pane(_) => return Err(Fail::NotATile),
                Node::Tile { pie, .. } => *pie,
            },
        };
        // **判的是"这一枚还能不能交出去"，与它是哪种资源无关。** 从前这里借 `vested_by`
        // 代理（那一问的正文是 `Reserve`，而 `Reserve` **只认孔**：对页与铃答 `Denied`）
        // ⇒ 页与铃当门牌时一律被判死，还顺手把那一格剔掉（真机量到：`land=Ok` 而
        // `find=Err(Dead)`，四枚砖全一样）。`Alive` 那一格答的正是这件事实。
        if !alive(pie) {
            let _ = self.unlink(id);
            let _ = pie::release(pie);
            return Err(Fail::Dead);
        }
        ship(pie);
        Ok(())
    }

    /// **开**：第 `id` 格**是谁的门牌**（那一枚句柄的开者）
    /// 与 Operator::find **正相反**：这一条什么都不交出去——只答"是谁"，不把那枚句柄递出去
    /// 三格，与 `find` 对同一格答**同一批码**（差别只在"活着的砖"那一档：`find` 交出句柄
    /// - 号不在树上（墓碑 / 从没铸过）⇒ Fail::Unknown
    /// - 那一格是一块 `Pane` ⇒ Fail::NotATile（**没有开者这一说**）
    /// - 是一枚 `Tile`，但开者那扇门封印了 ⇒ Fail::Dead
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

    /// **剪**：把那一号那一格剪掉
    /// 那一格得存在（否则 Fail::Unknown）；是 `Pane` 的话**必须空着**（否则 Fail::NonEmpty）
    /// 剪掉一枚 `Tile` 时放下本地引用（pie::forget），保留已交付的副本——它是资源实体的一份引用，不放下就漏水
    /// **剪掉的那一槽留成墓碑**（`None`），不 `remove`：号是下标，一移后面全错位。故一枚剪过的
    /// 号从此答 Fail::Unknown，而**它不会被重新铸出来**（水位只增）
    /// 答**真动了什么**：剪了"一枚空的 `Pane`"与"一枚 `Tile`"都算真动了树（前者是一格没了，
    /// 后者是一格连资源一起放下）——两档订阅者都该知道"这一号从此不在了"。剪不动的那两格
    /// （号不在 / 那块窗格非空）是 `Err`，与"没变"不是一件事。
    pub fn trim(&mut self, id: EntryId) -> Result<Option<Change>, Fail> {
        let dropped = match self.slot(id) {
            None => return Err(Fail::Unknown),
            Some(slot) => match &slot.node {
                Node::Pane(inner) if inner.is_empty() => None,
                Node::Pane(_) => return Err(Fail::NonEmpty),
                Node::Tile { pie, .. } => Some(*pie),
            },
        };
        // **路要在剪之前记**：剪完那一槽是碑，`road_to` 再也走不出来。
        let road = self.road_to(id);
        if let Some(pie) = dropped {
            if pies().any(|p| p.token == pie) {
                pie::forget(pie).map_err(|_| Fail::Unknown)?;
            }
        }
        let _ = self.unlink(id);
        // 剪了"一枚空的 `Pane`"与"一枚 `Tile`"都算真动了树：前者是一格没了，后者是一格连资源
        // 一起放下——两档订阅者都该知道"这一号从此不在了"。**路在落那条事件时现走**（号还在，
        // 槽已空）——故 `Change` 不存路。
        Ok(Some(Change {
            kind: Kind::Trimmed,
            id,
            owner: TaskId::new(0),
            road,
        }))
    }

    /// **列**：看那一块 `Pane` 里有哪些**号**（Where::Root = 根那一层）
    /// 号不在 ⇒ Fail::Unknown；那一号是一枚 `Tile` ⇒ Fail::NotAPane。**不过问死活**
    /// 剔死是 Operator::find 那一路上的事
    /// 答的是号、不是名字（机器用号，人用名——名字另问 Operator::name）
    /// 顺序是**号序**：pane 里本来是登记序，而号单调 ⇒ 两者一致，不必额外排
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

    /// **名**：这枚号此刻叫什么
    pub fn name(&self, id: EntryId) -> Result<String, Fail> {
        self.slot(id)
            .map(|slot| slot.name.clone())
            .ok_or(Fail::Unknown)
    }

    pub fn reference(&self, id: EntryId) -> Option<PieToken> {
        match &self.slot(id)?.node {
            Node::Tile { pie, .. } => Some(*pie),
            Node::Pane(_) => None,
        }
    }
    pub fn permit(&self, id: EntryId) -> Permit {
        match self.slot(id) {
            Some(Slot {
                node: Node::Tile { permit, .. },
                ..
            }) => *permit,
            _ => Permit::Public,
        }
    }

    /// **这一格此刻能不能重落**：无主 ⇒ 能；有主 ⇒ 只有主人自己，或**那一枚已经不在了**
    /// （`establish::alive`：不在表里 / 已封印）才轮得到别人。
    ///
    /// **判据与 `find` 同一格**（"这一枚还能不能交出去"与"这一格能不能换人"是同一件事）：
    /// 从前这里借 `vested_by`（孔那一套的父手），于是**页与铃那两格恒"可重落"**——任何域都能
    /// 把别人的门牌换绑成自己的。
    pub fn claimable(&self, key: Key<'_>, who: TaskId) -> bool {
        let Some(Slot {
            node: Node::Tile { pie, owner, .. },
            ..
        }) = self.slot_at(key)
        else {
            return true;
        };
        match owner {
            None => true,
            Some(owner) => *owner == who || !alive(*pie),
        }
    }

    /// 落 / 分共用的那一手：在 `at` 那一块 `Pane` 里给 `name` 立一格（或换绑那一格）
    /// **答那一格自己的号**：换绑不动号，故只有"真铸了一格"才动水位。**不递归**——
    /// `at` 那一块 `Pane` 就是一次 Operator::kids
    /// **先只读地问一遍**（那一格叫什么号），再动手：这样动手那一段只需要一次
    /// `slots[i]` 的可变借用，不必在 children 里穿一层 `&mut`（那正是老一版递归的由头）
    fn put(&mut self, location: Location, node: Node) -> Result<EntryId, Fail> {
        let Location { at, name } = location;
        let want = if matches!(node, Node::Tile { .. }) {
            Want::Tile
        } else {
            Want::Pane
        };
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
                if let Some(old) = old {
                    if pies().any(|p| p.token == old) {
                        pie::forget(old).map_err(|_| Fail::Unknown)?;
                    }
                }
                slot.node = node;
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

    /// 这一块 `Pane` 的孩子（Where::Root = 根那一层；`At(id)` = 那一格，得是 `Pane`）
    /// 三格判据与老一版的走法逐条对齐：号不在 ⇒ Fail::Unknown；那一格是 `Tile` ⇒
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

    /// 按号取那一格（只读）。号不在 / 是墓碑 ⇒ `None`
    fn slot(&self, id: EntryId) -> Option<&Slot> {
        self.slots.get(id.get()).and_then(Option::as_ref)
    }

    /// 按 Key 那两把钥匙取那一格（只读）。两种寻址**打的是同一格**（见 Key）
    fn slot_at(&self, key: Key<'_>) -> Option<&Slot> {
        match key {
            Key::Id(id) => self.slot(id),
            Key::At(at, name) => self.slot(self.child(self.kids(at).ok()?, name)?),
        }
    }

    /// 在这一块 `Pane` 的孩子里按名字找那个号（只读一趟扫，最多 `PANE_CAP` 次查表）
    fn child(&self, level: &[EntryId], name: &str) -> Option<EntryId> {
        level
            .iter()
            .copied()
            .find(|id| self.slot(*id).is_some_and(|slot| slot.name == name))
    }

    /// 按号把那**一格拿走**：槽标成墓碑（`None`），再把它从**某一个**父的 children 里摘掉
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

    /// **从根写起的那条路**：`id` 那一格自己那一段 ＋ 一路向上的每一段。
    ///
    /// 表里每一格只记**自己那一段名**（路那一层不在槽上，见 `Slot` 的注），故要整条路只能
    /// 从根走下来找它——`PANE_CAP` 与 `Path::MAX` 都是小常数，这条走法每层最多扫一趟。
    ///
    /// 返 `None` = 这一号不在树上（碑 / 从没铸过）：**事件里那条路拼不出来就不发**，
    /// 比发一条路是假的强。
    pub fn road_to(&self, id: EntryId) -> Option<PathBuf> {
        let mut segs: Vec<String> = Vec::new();
        let mut want = id;
        loop {
            // 找"哪一层的孩子里有 `want`"，把那一层的**父**继续往上带。
            let (parent, name) = self.locate(want)?;
            segs.try_reserve(1).ok()?;
            segs.push(name);
            match parent {
                Where::Root => break,
                Where::At(up) => want = up,
            }
        }
        // 从根写起 ⇒ 反着拼。
        let mut road = PathBuf::from(Path::ROOT);
        for seg in segs.iter().rev() {
            let next = road.try_join(seg.as_str())?;
            road = next;
        }
        Some(road)
    }

    /// `id` 那一格住在哪：**父那一块 `Pane`（根那一层报 `Where::Root`）＋ 它自己那一段名**。
    fn locate(&self, id: EntryId) -> Option<(Where, String)> {
        if self.root.contains(&id) {
            let name = self.slot(id)?.name.clone();
            return Some((Where::Root, name));
        }
        for i in 0..self.slots.len() {
            let Some(Some(one)) = self.slots.get(i) else {
                continue;
            };
            let Node::Pane(inner) = &one.node else {
                continue;
            };
            if inner.contains(&id) {
                let name = self.slot(id)?.name.clone();
                return Some((Where::At(EntryId::new(i)), name));
            }
        }
        None
    }
}

// 真正的对照表只有一份（protocol::system::operator::frame）；`gate` 为了"不带载体"自己
// 看得见 `gate` 与 `frame`。
const _: () = {
    use protocol::system::operator::frame;
    assert!(gate::WIRE_OK == frame::OK);
    assert!(gate::WIRE_DENIED == frame::DENIED);
    assert!(gate::WIRE_UNJUDGED == frame::UNJUDGED);
};
