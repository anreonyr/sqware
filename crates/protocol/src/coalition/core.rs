//! coalition::core — **盟册**：一条关系、一枚计数器、六条原语。
//!
//! 本文件**不 `use` 内核**——连 `TaskId` 都不认识（喂两个假号就能把六条规矩推理干净）。
//! 这不是风格：身份那一侧归 [`principal`](crate::principal)（名册把内核盖的章翻成一条号），
//! 本册只收**一条已经解析好的身份**；故"这条请求是不是发送者本人"不在这一层——它在适配层
//! （见正文"已知边界"）。
//!
//! ```text
//!   盟籍   PrincipalId ──→ CoalitionId     这条身份在哪些盟里     bloc(p)   核心
//!          CoalitionId ──→ PrincipalId     这枚盟里有谁           band(c)   核心
//!   号     0 .. next                    铸过就一直在（没有墓碑、没有 id 表）
//! ```
//!
//! 两条轴的分工见正文（[`super`]）：**横向的盟籍**只有一张表，反着念就是第二个方向。
//! 六条原语里只有三条动数据（`found` / `enter` / `leave`）。

use alloc::vec::Vec;

use crate::principal::core::PrincipalId;

// ── 号 ──────────────────────────────────────────────────────

/// 盟册上的一枚号。
///
/// **裸号**：与 [`PrincipalId`] 同形（8 字节、小端上线），但**不同源**——两个号空间互相拿错正是
/// 旧树栽过的那一格。故"这枚号铸过没有"不是类型义务，是每条读查一次计数答出来的
/// [`Fail::Unknown`]；[`CoalitionId::new`] 造得出任何号，那正是探针验第三态的路子。
///
/// **没有 `ROOT`**：盟无根、无主——零号是一枚**普通的盟**（对照 [`PrincipalId::ROOT`]：
/// 那是身份那一侧"唯一没有父的节点"）。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct CoalitionId(usize);

impl CoalitionId {
    /// 由裸号造一个（线上解码面；没铸过的号从这里进来）。
    pub const fn new(raw: usize) -> CoalitionId {
        CoalitionId(raw)
    }

    /// 裸号。
    pub const fn get(self) -> usize {
        self.0
    }

    /// 线上的那一格（8 字节小端）。
    pub const fn to_bytes(self) -> [u8; 8] {
        (self.0 as u64).to_le_bytes()
    }

    /// 由线上字节还原（**不校验**：铸过没有由核心答）。
    pub const fn from_bytes(bytes: [u8; 8]) -> CoalitionId {
        CoalitionId(u64::from_le_bytes(bytes) as usize)
    }
}

// ── 失败域 ──────────────────────────────────────────────────

/// 失败域：**两格**，每格一个**不同的下一步**。
///
/// **没有 `Denied`**——本族没有一处"你得请谁来做"的判断：盟无主，四条写里的门要么是
/// "这条号是假的"，要么是"备不下"。这是横向那条轴与纵向那条轴（[`principal`](crate::principal)
/// 有 `Denied`）在失败域上的分野。
///
/// **读那两条里只有 `band` 有失败域**：`bloc` 答不出"查无此籍"——`p` 是别人给的标签，
/// 本册不去问身份服务，不在任何盟里就是空串。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    /// 这枚盟不存在（从来没铸过），或这个 TID 没绑过。调用方要改的是：**我手里这个号是假的**
    /// 或**我还没有身份**。
    Unknown,
    /// `try_reserve` 备不下。调用方要改的是：**晚点再来**。
    ///
    /// **只有 [`Coalition::enter`] 到得了这一格**：`found` 不分配（只动计数器），
    /// 两条转换与两条读也不分配。
    NoRoom,
}

// ── 一窗号 ──────────────────────────────────────────────────

/// 一窗最多几枚号。条数是策略、容器要有界 ⇒ 窗口有顶，**"还有没有"由 `more` 说**。
pub const WINDOW_CAP: usize = 16;

/// 一条号：窗口只问这两下（由裸号造、看裸号是多少）。
///
/// **`new` 不校验**（线上解码面造得出任何号）："这枚号还在不在"不是类型义务——由每册自己的
/// 判据答（盟那一册是 [`Coalition::band`] 的 [`Fail::Unknown`]）。
pub trait Id: Copy {
    /// 由裸号造一个。
    fn new(raw: usize) -> Self;

    /// 裸号。
    fn get(self) -> usize;
}

impl Id for CoalitionId {
    fn new(raw: usize) -> CoalitionId {
        CoalitionId::new(raw)
    }

    fn get(self) -> usize {
        CoalitionId::get(self)
    }
}

impl Id for PrincipalId {
    fn new(raw: usize) -> PrincipalId {
        PrincipalId::new(raw)
    }

    fn get(self) -> usize {
        PrincipalId::get(self)
    }
}

/// 一窗号：**一趟读的读数**（最多 [`WINDOW_CAP`] 枚，**号序升序**）。
///
/// 空位是 `None` 而不是 `T::new(0)`：**零号是真格子**（[`PrincipalId::ROOT`] 就是 0），
/// 拿它当"这一格空着"正是要避开的那件事。
///
/// **取窗落在核心**（[`Coalition::band`] / [`Coalition::bloc`] 扫一遍表就填出来）：服务那一层
/// 只把它编成帧，不做选择。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window<T: Id> {
    items: [Option<T>; WINDOW_CAP],
    n: usize,
    more: bool,
}

impl<T: Id> Window<T> {
    /// 空的那一串（`more = false`）。
    pub const fn new() -> Window<T> {
        Window {
            items: [None; WINDOW_CAP],
            n: 0,
            more: false,
        }
    }

    /// 由一串号凑一窗（`more` = 窗外还有）——**解码面**：线上收来的那一窗由这里成形。
    ///
    /// 收够 [`WINDOW_CAP`] 枚就停：帧长了是帧的毛病，读的人只认窗前这些（帧长与条数对不对
    /// 由 [`call`](super::call) 那一层先挡掉）。
    pub fn gather(more: bool, ids: impl Iterator<Item = T>) -> Window<T> {
        let mut out = Window::new();
        for id in ids.take(WINDOW_CAP) {
            out.push(id);
        }
        out.more = more;
        out
    }

    /// 几枚。
    pub fn len(&self) -> usize {
        self.n
    }

    /// 一枚都没有。
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// 窗外还有没有（这一趟没答完的那些）。
    pub fn more(&self) -> bool {
        self.more
    }

    /// 第 `at` 枚（号序；越界 ⇒ `None`）。
    pub fn get(&self, at: usize) -> Option<T> {
        if at < self.n {
            self.items.get(at).copied().flatten()
        } else {
            None
        }
    }

    /// 号序走一遍。
    pub fn iter(&self) -> impl Iterator<Item = T> + '_ {
        self.items[..self.n].iter().filter_map(|slot| *slot)
    }

    /// 末一枚——**它就是下一页的游标**（空窗 ⇒ `None`）。
    pub fn last(&self) -> Option<T> {
        self.n.checked_sub(1).and_then(|at| self.get(at))
    }

    /// 收一枚（再满就丢：取窗那边收了 [`WINDOW_CAP`] 枚就停）。
    fn push(&mut self, id: T) {
        if let Some(slot) = self.items.get_mut(self.n) {
            *slot = Some(id);
            self.n += 1;
        }
    }

    /// 装满了。
    fn full(&self) -> bool {
        self.n == WINDOW_CAP
    }
}

// ── 一格盟籍 ────────────────────────────────────────────────

/// 盟籍一格：**一对号，没有第三格**。
///
/// 没有角色、没有权重、没有序位——"盟只是一组身份"这句话就落在这里：**类型里写不出别的**。
/// 唯一性（同一对不许出现两次）类型表达不了，落在 [`Coalition::enter`] 的**先查后推**上
/// （与 `Principal::bind` 覆盖那一趟同一形状）：**一次线性扫，换掉"两张表要同步"那条义务**。
struct Ally {
    who: PrincipalId,
    of: CoalitionId,
}

// ── 状态 ────────────────────────────────────────────────────

/// 盟册的权威状态：**一条关系 + 一枚计数器**。
///
/// **没有钥匙那一格**（对照 `Principal.assembler`）：盟无主——"无主"这条裁定的落点，
/// 就是一格不存在的字段。
pub struct Coalition {
    /// 号的存在那一格：**号 < `next` 即铸过**。只增（全文件没有一处减它）。
    ///
    /// 一枚计数器而不是一张表：每枚盟没有任何内容可存（无父、无名、无主），存下来就是一格
    /// 空行——`found` 因此**不分配**，也到不了 [`Fail::NoRoom`]。
    next: usize,
    /// 盟籍：一格一对号，可增可删。
    book: Vec<Ally>,
}

impl Coalition {
    /// 立一份空册：**一枚号都还没铸**（零号也还没有）。
    ///
    /// 不分配 ⇒ 不失败（对照 `Principal::new`：它要为根 `try_reserve`）。
    pub const fn new() -> Coalition {
        Coalition {
            next: 0,
            book: Vec::new(),
        }
    }

    // ── 立 ──────────────────────────────────────────────────

    /// 盟 · 写：立一枚新号。号取当前计数，再把计数加一。
    ///
    /// **不分配、不校验 ⇒ 没有失败域**——全族唯一一条不可失败的原语。钥匙（发送者得是个
    /// 已绑定的身份）在**适配层**：核心不认识 TID，也不持名册，故它解析出来的那条号在本层
    /// 只当门卫、不落表（盟无主，不记铸造者）。
    ///
    /// 号因此单调、稠密、"铸过就一直在"；空盟合法且免费（没有墓碑，也没有 id 表）。
    pub fn found(&mut self) -> CoalitionId {
        let id = CoalitionId(self.next);
        self.next += 1;
        id
    }

    // ── 入 / 出 ─────────────────────────────────────────────

    /// 盟 · 写：把 `who` 放进 `c`。**幂等**：已在里面答 `Ok(())`、表不动——集合没有"第二次"。
    ///
    /// 两格前置各问一件事：`c` 铸过没有（[`Fail::Unknown`]）、备得下那一行吗（[`Fail::NoRoom`]）。
    ///
    /// `who` 由适配层给：**核心收的是"一条已经解析好的身份"**，故"是不是发送者本人"这一格
    /// 不在核心（见正文"已知边界"）。
    pub fn enter(&mut self, who: PrincipalId, c: CoalitionId) -> Result<(), Fail> {
        if !self.stands(c) {
            return Err(Fail::Unknown);
        }
        // 先查后推：一对号只许有一格（集合语义）。
        if self.book.iter().any(|a| a.who == who && a.of == c) {
            return Ok(());
        }
        self.book.try_reserve(1).map_err(|_| Fail::NoRoom)?;
        self.book.push(Ally { who, of: c });
        Ok(())
    }

    /// 盟 · 写：把 `who` 从 `c` 拿出来。**撞空也成**（`Ok(())`）——集合运算没有"第二次"，
    /// 与 [`Coalition::enter`] 同一条幂等律。
    ///
    /// **照实记：这一格与 `Principal::unbind` 的撞空不同**——那边答 [`Fail::Unknown`]，
    /// 因为名册是**账**（"这一格本来就没有"是一条值得如实回答的事实，撤一格是记账动作）；
    /// 这里是**集合**，进与出都是集合运算。
    pub fn leave(&mut self, who: PrincipalId, c: CoalitionId) -> Result<(), Fail> {
        if !self.stands(c) {
            return Err(Fail::Unknown);
        }
        self.book.retain(|a| !(a.who == who && a.of == c));
        Ok(())
    }

    // ── 在 / 员 / 籍 ────────────────────────────────────────

    /// 盟 · 读：`p` 在不在 `c` 里。**两件事两个落点**——`Ok(false)` 是诚实的答案（不在），
    /// [`Fail::Unknown`] 是"这枚盟不存在"。
    ///
    /// `p` 是不是真身份与它无关：`p` 是标签。故这条读**不过名册**（线上唯一那条读也不需要
    /// 依赖身份服务），第三态只有"查无此盟"一格。
    pub fn amid(&self, p: PrincipalId, c: CoalitionId) -> Result<bool, Fail> {
        if !self.stands(c) {
            return Err(Fail::Unknown);
        }
        Ok(self.book.iter().any(|a| a.who == p && a.of == c))
    }

    /// 盟 · 读：`c` 里此刻有谁（**一趟取窗**）。
    ///
    /// 序 = **号序升序**（不是登记序）；`after` 是**阈值**——取号 > 它的那些，`None` = 从头取。
    /// 零号是真格子（[`PrincipalId::ROOT`] 就是 0），故**不能用 0 当"没有游标"**：有没有由
    /// `Option` 说。
    ///
    /// 窗装不下 ⇒ [`Window::more`] 为真，要接着取就把**末一枚**当下一趟的 `after`。
    /// **照实记（代价）**：两次取窗之间表变了 ⇒ 跨页**只会漏，不会重**（阈值单调，而新进的那些
    /// 可能落在已走过的阈值之下）——故没有"过期游标"这回事，`cookieverf` 那一格不需要。
    ///
    /// 只有一格失败：这枚盟没铸过（[`Fail::Unknown`]）。
    pub fn band(
        &self,
        c: CoalitionId,
        after: Option<PrincipalId>,
    ) -> Result<Window<PrincipalId>, Fail> {
        if !self.stands(c) {
            return Err(Fail::Unknown);
        }
        Ok(self.window(|a| a.of == c, |a| a.who, after))
    }

    /// 盟 · 读：`p` 此刻在哪些盟里（**一趟取窗**，序与游标同 [`Coalition::band`]）。
    ///
    /// **没有失败域**——`p` 是标签，不在任何盟里就是空窗。
    ///
    /// 这一条的不对称写进了签名：[`Coalition::band`] 答 `Result`（它问的是**本册自己的**
    /// 号空间，故有"查无此盟"），`bloc` 不答（它问的是**别人的**号空间，本册不去问）。
    pub fn bloc(&self, p: PrincipalId, after: Option<CoalitionId>) -> Window<CoalitionId> {
        self.window(|a| a.who == p, |a| a.of, after)
    }

    /// 一趟取窗：从 `book` 里挑相符的那些，按**号序**取「大于 `after` 的前 [`WINDOW_CAP`] 枚」。
    ///
    /// **零分配的选择扫**：表是登记序，每取一枚都要重扫一遍挑"比上一枚大的里头最小的那个"
    /// （`O(条数 × 窗宽)`，窗宽封顶 16）。要它成对：多扫一趟就为答 [`Window::more`]——
    /// 那正是"还有没有"的读数，不能靠"表里还有几行"猜。
    fn window<K: Id>(
        &self,
        keep: impl Fn(&Ally) -> bool,
        key: impl Fn(&Ally) -> K,
        after: Option<K>,
    ) -> Window<K> {
        let mut out = Window::new();
        let mut last = after;
        loop {
            let mut best: Option<K> = None;
            for ally in &self.book {
                if !keep(ally) {
                    continue;
                }
                let at = key(ally);
                if let Some(mark) = last {
                    if at.get() <= mark.get() {
                        continue;
                    }
                }
                match best {
                    Some(seen) if seen.get() <= at.get() => {}
                    _ => best = Some(at),
                }
            }
            match best {
                Some(at) if !out.full() => {
                    out.push(at);
                    last = Some(at);
                }
                // 窗外还有 ⇒ 这一趟到此为止，"未完"是读数的一部分。
                Some(_) => {
                    out.more = true;
                    break;
                }
                None => break,
            }
        }
        out
    }

    /// 这枚号铸过没有。**只有这一处读 `next`**——存在性的判据只此一份。
    fn stands(&self, c: CoalitionId) -> bool {
        c.get() < self.next
    }
}

// ── 用例不在这里（照实记：用户裁定"测试和运行环境分开"）──────────────
//
// 本文件原先那个 `#[cfg(test)] mod tests`（**6 条**）整体搬去了 `crates/protocol-case` 的
// `roster` 靶里 `coalition_core` 那一格（盟籍要 `crate::principal::core` 的号，故与名册
// 同住一个靶），门口 `crates/gate/tests/host.rs`；**本文件从此没有一行测试**。
//
// 那一批原先是"编不到、也跑不到"的规格（`protocol` 是 `[lib] test = false`）。真机上另有
// 探针那几条（`harness/src/member.rs`）。
