//! operator::ledger —— **那一本账**：一格的两轴事实（谁许用 / 归谁改）。**不带载体。**
//!
//! 本文件与 [`judge`](super::judge) / [`gate`](super::gate) 同一站位：只判、只记，**不发消息**。
//! 它住在协议里而不在服务里的理由全是实测的：这本账的前身（`server.rs` 的 `Publishers`）住在
//! 适配层，**编不进宿主靶**——而它已经在真机上量错过两次（第一版按号记 ⇒ 整道判据被跳过，
//! `probe-owner` 顶掉了 `/device/uart` 的牌子；`Desk` 的定长数组 ⇒ 撞满三次）。
//!
//! # 一条记录，两把钥匙
//!
//! ```text
//!   land   手里有 (坐标, 名)      ─┐
//!   find   手里有 号              ─┼─→  同一条 [Line]
//!   trim   手里有 号              ─┘
//! ```
//!
//! 两种寻址打的是**同一格**——这条不写成结构，就会像第一版那样反复在两处各补一次补丁
//! （按号记 ⇒ `land` 查不到；改按坐标记 ⇒ `find`/`trim` 又要赔一趟全树递归）。
//!
//! # 账是**缓存**，树才是真相
//!
//! 格子从树上有**三条路**会消失，而树一个字都不知道这本账：
//!
//! | 路 | 谁干的 |
//! |---|---|
//! | `trim` | 客人 |
//! | [`find`](super::core::Operator::find) 的**惰性剔死** | 持树者自己（那一枚答不出就当场剔掉） |
//! | `part` **顶掉**一枚 `Tile`（[`Operator::part`](super::core::Operator::part) 那条照实记） | 客人 |
//!
//! 想把"账 ⊆ 树"当不变量，就得给树装三个回调——而**"树不知道规矩"是已定的边界**。
//! 故这里换一条：**查的时候对一次真相**（[`Ledger::rule`] / [`Ledger::claimable`] 收的那个
//! `fresh` 闭包），对不上就**顺手销账**。
//!
//! 这与仓里已经跑通的**两条判据形状相同**：`find` 的惰性剔死、`claimable` 的"主人不在场"
//! ——都是"**不问就不动，问了才发现**"。这里是第三处。
//!
//! **顺带的一格**：号不复用（"树只增不删 ⇒ 号不会失效"），而**坐标复用**。故陈旧只可能撞在
//! 坐标那一侧；按号查出来的陈账，只可能被"对同一个号的又一次操作"撞上，而那一次操作本身会在
//! 树上答 `Unknown` / `NotATile`——但门口的裁决发生在动树之前，所以两侧都走同一条查法。
//!
//! # 三样东西的分工
//!
//! ```text
//!   账（本文件）  答"这一格的规矩是几、归谁"
//!   judge         答"按这条规矩，这一位许不许"
//!   gate          答"这一问该给客人回哪一格码"
//! ```

use alloc::vec::Vec;

use env::{Name, PieToken, TaskId};

use super::core::{EntryId, Fail, VestedBy, Where};
use super::judge::Rule;

// ── 两把钥匙 ────────────────────────────────────────────────

/// **这一格**的两种报法——两种寻址打的是同一格。
///
/// 这不是"省一条查法"：坐标是 `land` 那一问手里唯一的凭据（那时号还不存在或不必知道），
/// 而号是 `find` / `trim` 手里唯一的凭据（那时坐标早不知道了）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    /// 坐标：那一块 `Pane` + 那一段名字（`land` 那一问手里有的）。
    At(Where, Name),
    /// 号：条目自己的号（`find` / `trim` 手里有的）。
    Id(EntryId),
}

// ── 一格的一行 ──────────────────────────────────────────────

/// 那一行记的**主人**：谁落的牌、以及**那一刻挂上去的那一枚**。
///
/// 后一格是这一轴的全部护栏：它答不答得出（[`VestedBy`]），就是主人还在不在场。
///
/// **为什么键是"命"不是"身份"**（照实记：这一格从 `f4f0da9` 一直悬着，裁在后来"两轴分家"
/// 那一刀）——三条理由，一条比一条硬：
///
/// 1. **键与护栏必须同级**：这一轴唯一的护栏是"**主人还在不在场**"（[`VestedBy`]），而**封印
///    是按任务来的**（退场钩子封印该域开的资源）。若键改成身份，护栏就得问"**那一条身份**
///    还在不在场"——而身份**永不消亡**（`principal` 的节点只增不删）⇒ 那条护栏当场失去意义。
/// 2. **`mine = true` 是一个动作的产物**："**我**落牌那一刻声明这一格归我"——主语是任务。
/// 3. **今天没有客人要那另一种**（"另一枚 TID 代表同一条身份也能改得动"）。
///
/// **被否**：把 `who` 换成身份号。代价两条：① 写那一侧要多问一次名册（`claimable` 之前先换
/// 身份），而"用"那一侧才刚为 [`Rule::Opens`] 破过一次"只问一次"；② 键与护栏不同源（理由 1）。
/// 换来的是"与用那一轴同键"这点形式上的整齐——不值。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Owner {
    who: TaskId,
    pie: PieToken,
}

/// 账上的一行：**一格的两轴事实**。
///
/// - `rule` = **用**那一轴（谁许用这一格）；
/// - `owner` = **改**那一轴（谁许改这一格；`None` = 从没声明过归属 ⇒ 谁都能落）。
///
/// 两格都必要：把它们混成一格，就会得出"能改的人自然能用"（而反过来才是常见的那一种）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Line<P, C> {
    at: Where,
    name: Name,
    id: EntryId,
    rule: Rule<P, C>,
    owner: Option<Owner>,
}

impl<P, C> Line<P, C> {
    /// 记一行。`mine = false` ⇒ **不留主人**（连"放弃"也走这一条：重绑一次就换人了）。
    pub const fn new(
        at: Where,
        name: Name,
        id: EntryId,
        rule: Rule<P, C>,
        mine: bool,
        who: TaskId,
        pie: PieToken,
    ) -> Self {
        Self {
            at,
            name,
            id,
            rule,
            owner: if mine { Some(Owner { who, pie }) } else { None },
        }
    }
}

/// **预留的空白一行**：拿了它才写得进账（字段私有 ⇒ 外面造不出来）。
///
/// 它存在的唯一理由是**次序**：`land` 必须先 `grow`（要位）、再动树、最后 `write`。
///
/// 照实记（为什么不省掉它）：反过来（先动树后记账）一旦记账失败，就是"树改了、账没记上"
/// ⇒ 私名变公名（**fail-open**）；而先记账又拿不到号（号是 `land` 答的）。故只能先要位。
/// 没有这一格时，"先要位"就只是一句口头契约——漏了不会响。
#[derive(Debug)]
pub struct Blank(());

// ── 那本账 ─────────────────────────────────────────────────

/// **那一本账**：一格一条，按两种钥匙查。
///
/// 它的"活着"那一问与树要的是**同一句**（[`VestedBy`]：那一枚还答得出吗），故这里直接复用
/// 内核注入树的那一枚函数指针，不另开一个 trait。
pub struct Ledger<P, C> {
    lines: Vec<Line<P, C>>,
    vested_by: VestedBy,
}

impl<P: Copy + PartialEq, C: Copy> Ledger<P, C> {
    /// 起一本空账。`vested_by` 与 [`Operator::new`](super::core::Operator::new) 收的是同一枚。
    pub const fn new(vested_by: VestedBy) -> Self {
        Self {
            lines: Vec::new(),
            vested_by,
        }
    }

    /// 这一格**谁许用**。
    ///
    /// 账上没有 ⇒ [`Rule::Public`]（`part` 出来的格子本来就没有规矩）；那一行已陈旧 ⇒ 同上，
    /// 且**顺手销掉**。**两条都不是失败**——门口那一问没有"我判不了"的余地：判不了要留给
    /// 问身份那两条边的失败（[`Ruling::Unjudged`](super::judge::Ruling::Unjudged)）。
    pub fn rule(&mut self, key: Key, fresh: impl Fn(EntryId) -> bool) -> Rule<P, C> {
        match self.look(key, &fresh) {
            Some(at) => self.lines[at].rule,
            None => Rule::Public,
        }
    }

    /// 这一格**归不归 `who` 改**。
    ///
    /// 四支，每一支一个理由：
    ///
    /// - 账上没有 ⇒ **可以**（没声明过归属；`part` 分出来的格子也走这一支）；
    /// - 就是他 ⇒ **可以**（主人改自己的格子，包括用 `mine = false` 放弃）；
    /// - 主人**不在场**（那一枚答不出）⇒ **可以**——这就是"规矩属于**活着的**主人"；
    /// - 那一行**已陈旧** ⇒ **可以**（树上那一格已经不是它了；真相由树去答）。
    pub fn claimable(&mut self, key: Key, who: TaskId, fresh: impl Fn(EntryId) -> bool) -> bool {
        let Some(at) = self.look(key, &fresh) else {
            return true;
        };
        match self.lines[at].owner {
            None => true,
            Some(owner) if owner.who == who => true,
            Some(owner) => (self.vested_by)(owner.pie).is_none(),
        }
    }

    /// 找那一行；**陈旧的那一行顺手销掉**（账是缓存，树才是真相）。
    ///
    /// `fresh` 只会在这一处被叫——而这一处只在"要拒"的支上真的紧（`rule` 那一支是纯查，
    /// 但同一个 `look` 走两条路，故代价合在一处说）。
    fn look(&mut self, key: Key, fresh: &impl Fn(EntryId) -> bool) -> Option<usize> {
        let at = self.lines.iter().position(|line| match key {
            Key::At(at, name) => line.at == at && line.name == name,
            Key::Id(id) => line.id == id,
        })?;
        if fresh(self.lines[at].id) {
            Some(at)
        } else {
            // 树上那一格已经换了人/没了 ⇒ 这一行作废。**不是错误，是账对不上真相。**
            let _ = self.lines.remove(at);
            None
        }
    }

    /// **先要位**：把这一行的容量腾出来。
    ///
    /// 失败 ⇒ 这一问**就该失败**（答 `FULL`）。理由：这本账的"用"那一轴一旦漏记，那一格就
    /// 从"私名"回落成"公名"——记账失败**不能**悄悄放过。
    ///
    /// **照实记**：前身（`Publishers`）在这里是 fail-soft（`if try_reserve(1).is_ok()`）。
    /// 同一个 fail-soft 在两轴上后果不同：改那一轴记不上只是少一条归属账，用那一轴记不上是
    /// **开的**。这就是这一格从"顺手修"升成"必需"的原因。
    pub fn grow(&mut self) -> Result<Blank, Fail> {
        self.lines.try_reserve(1).map_err(|_| Fail::Full)?;
        Ok(Blank(()))
    }

    /// 记一行（重绑 = 改写同一条）。
    ///
    /// **找同一条用的是两个键的 OR**（`id` 相等 **或** 坐标相等）——因为写的三条路手里拿的
    /// 东西不同：`land` 拿的是 (坐标, 名)，重绑时 `id` 不变（改写同一条）；`part` / `trim` /
    /// 剔死拿的是 `id`（各自 [`Ledger::drop`] 销账）。
    ///
    /// **照实记（为什么两个键不可能指向不同的行）**：树里 (坐标,名) 与 `id` **一一对应**，而
    /// 号不移动（`land` 重绑不动号；`trim` 把号留成墓碑，不回收）⇒ 一条活行两个键都指它。
    /// 收尾那一半住在**调用方**：`land` 覆写同一条、`part` / `trim` / 剔死各销一次账
    /// （`programs/src/system/operator/server.rs`）。**这本账自己不验**这件事——
    /// [`Ledger::look`] 的新鲜度只问 `fresh(id)`（"那一号还在树上吗、还是不是砖"），不问
    /// "它还在那个坐标下吗"；把那条义务也搬进来，`fresh` 的签名就得带上 (坐标, 名)。
    ///
    /// **契约：先 [`Ledger::grow`] 过，故它不失败**（容量已经腾出来了）。这条契约由 [`Blank`]
    /// 那一格挡着：没要位就写，**编不过**。
    ///
    /// 照实记（`mine = false` 重绑 = 放弃归属）：前身只在 `mine` 为真时才记，旧记录会**永久
    /// 留着**——"改那一轴"因此没有"放弃"这一手。这一刀让它有：主人用一次 `mine = false` 重绑
    /// 就是声明"这一格不归我了"。走得到这一支的只有主人本人或接手者（前面拦着 `claimable`）。
    pub fn write(&mut self, _: Blank, line: Line<P, C>) {
        match self
            .lines
            .iter_mut()
            .find(|slot| slot.id == line.id || (slot.at == line.at && slot.name == line.name))
        {
            Some(slot) => *slot = line,
            None => self.lines.push(line),
        }
    }

    /// 销一行。**格子从树上消失的任一条路都该顺手叫它**（`trim` / `find` 答 `Dead` /
    /// `part` 顶掉一枚 `Tile`）——但它**不是正确性的一半**：漏了也不会答错（[`Ledger::look`]
    /// 那一次对真相兜着），只是这本账会留一条陈的。
    pub fn drop(&mut self, id: EntryId) {
        if let Some(at) = self.lines.iter().position(|line| line.id == id) {
            let _ = self.lines.remove(at);
        }
    }

    /// 账上有几行。
    ///
    /// **照实记（谁是这一格的读者）**：**宿主靶**——`protocol-case` 的 `judge` 靶拿它数"重绑
    /// 不该多出一条"/"失败不留半个状态"（8 处）。**生产路径不用它**（持树者问的是 `rule` /
    /// `claimable`："这一格什么规矩、归谁改"）。**它是靶的读数面**：靶 `#[path]` 逐字编源文件，
    /// 故这一格非 `pub` 不可；而"账上几行"也没有别的门问得出来（另一条读 `rule` 答的是规矩，
    /// 不是行数）——故留着它，并把读者写在这里。
    pub fn len(&self) -> usize {
        self.lines.len()
    }
}
