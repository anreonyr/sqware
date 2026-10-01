//! coalition::core — **盟册那一本账**：一条关系 ＋ 一条主子 ＋ 一枚计数器。

use alloc::vec::Vec;

use protocol::id::Id;
use protocol::service::coalition::{CoalitionId, Fail, Window};
use protocol::service::principal::PrincipalId;

/// 盟籍一格：**一对号，没有第三格**（K2 没翻的那一半）。
/// 没有角色、没有权重、没有序位——"盟只是一组身份"这句话就落在这里：**类型里写不出别的**。
/// 唯一性（同一对不许出现两次）类型表达不了，落在 [`Coalition::enter`] 的**先查后推**上
/// （与 `Principal::bind` 覆盖那一趟同一形状）：**一次线性扫，换掉"两张表要同步"那条义务**。
struct Ally {
    who: PrincipalId,
    of: CoalitionId,
}

/// **一格盟主**：这一枚盟是谁立的。
/// 它只有一个读者——[`Coalition::admit`] 那一句"代报名这件事归不归你"。
struct Chief {
    of: CoalitionId,
    chief: PrincipalId,
}

/// 盟册的权威状态：**一条关系 ＋ 一条主子 ＋ 一枚计数器**。
pub struct Coalition {
    /// 号的存在那一格：**号 < `next` 即铸过**。只增（全文件没有一处减它）。
    /// 一枚计数器而不是一张表：每枚盟除了"谁是主"没有任何内容可存（无父、无名），
    /// 存下来就是一格空行。
    next: usize,
    /// 盟籍：一格一对号，可增可删。
    book: Vec<Ally>,
    /// 盟主：一枚盟一行（只增——盟没有"解散"，见正文"已知边界"）。
    chiefs: Vec<Chief>,
}

impl Coalition {
    /// 立一份空册：**一枚号都还没铸**（零号也还没有）。
    /// 不分配 ⇒ 不失败（对照 `Principal::new`：它要为根 `try_reserve`）。
    pub const fn new() -> Coalition {
        Coalition {
            next: 0,
            book: Vec::new(),
            chiefs: Vec::new(),
        }
    }

    /// 盟 · 写：立一枚新号，并记下**立它那一位**（盟主）。号取当前计数，再把计数加一。
    pub fn found(&mut self, who: PrincipalId) -> Result<CoalitionId, Fail> {
        self.chiefs.try_reserve(1).map_err(|_| Fail::Full)?;
        let id = CoalitionId::new(self.next);
        self.next += 1;
        self.chiefs.push(Chief { of: id, chief: who });
        Ok(id)
    }

    /// 盟 · 写：把 `who` 放进 `c`。**幂等**：已在里面答 `Ok(())`、表不动——集合没有"第二次"。
    /// 两格前置各问一件事：`c` 铸过没有（[`Fail::Unknown`]）、备得下那一行吗（[`Fail::Full`]）。
    pub fn enter(&mut self, who: PrincipalId, c: CoalitionId) -> Result<(), Fail> {
        if !self.stands(c) {
            return Err(Fail::Unknown);
        }
        // 先查后推：一对号只许有一格（集合语义）。
        if self.book.iter().any(|a| a.who == who && a.of == c) {
            return Ok(());
        }
        self.book.try_reserve(1).map_err(|_| Fail::Full)?;
        self.book.push(Ally { who, of: c });
        Ok(())
    }

    /// 盟 · 写：把 `who` 从 `c` 拿出来。**撞空也成**（`Ok(())`）——集合运算没有"第二次"，
    /// 与 [`Coalition::enter`] 同一条幂等律。
    pub fn leave(&mut self, who: PrincipalId, c: CoalitionId) -> Result<(), Fail> {
        if !self.stands(c) {
            return Err(Fail::Unknown);
        }
        self.book.retain(|a| !(a.who == who && a.of == c));
        Ok(())
    }

    /// 盟 · 写：把 `target` 放进 `c`——**只有这一枚盟的盟主叫得动**。
    /// 这就是"替别人入盟"唯一那条路：`enter` 的钥匙是
    /// **发送者那一格**（只表达得了"我进"），故"许某一位进这一类"只能由立盟那位说。
    /// 三格前置：`c` 铸过没有（[`Fail::Unknown`]）、**你是不是这一枚的盟主**
    /// （[`Fail::NotChief`]）、备得下那一行吗（[`Fail::Full`]，由 [`Coalition::enter`] 答）。
    /// **`chief` 由适配层解**（同 `found`：核心收的是已经解析好的身份）；**`target` 也是**
    /// ——帧里那一格是 TID，名是服务端过名册点的。
    pub fn admit(
        &mut self,
        chief: PrincipalId,
        c: CoalitionId,
        target: PrincipalId,
    ) -> Result<(), Fail> {
        if !self.stands(c) {
            return Err(Fail::Unknown);
        }
        if self.chief_of(c) != Some(chief) {
            return Err(Fail::NotChief);
        }
        self.enter(target, c)
    }

    /// 这一枚盟是谁立的（没铸过 ⇒ `None`）。**只有 [`Coalition::admit`] 读它。**
    fn chief_of(&self, c: CoalitionId) -> Option<PrincipalId> {
        self.chiefs
            .iter()
            .find(|row| row.of == c)
            .map(|row| row.chief)
    }

    /// 盟 · 读：`p` 在不在 `c` 里。**两件事两个落点**——`Ok(false)` 是诚实的答案（不在），
    /// [`Fail::Unknown`] 是"这枚盟不存在"。
    /// `p` 是不是真身份与它无关：`p` 是标签。故这条读**不过名册**（线上唯一那条读也不需要
    /// 依赖身份服务），第三态只有"查无此盟"一格。
    pub fn amid(&self, p: PrincipalId, c: CoalitionId) -> Result<bool, Fail> {
        if !self.stands(c) {
            return Err(Fail::Unknown);
        }
        Ok(self.book.iter().any(|a| a.who == p && a.of == c))
    }

    /// 盟 · 读：`c` 里此刻有谁（**一趟取窗**）。
    /// 序 = **号序升序**（不是登记序）；`after` 是**阈值**——取号 > 它的那些，`None` = 从头取。
    /// 零号是真格子（[`PrincipalId::ROOT`] 就是 0），故**不能用 0 当"没有游标"**：有没有由
    /// `Option` 说。
    /// 窗装不下 ⇒ [`Window::more`] 为真，要接着取就把**末一枚**当下一趟的 `after`。
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
    /// **没有失败域**——`p` 是标签，不在任何盟里就是空窗。
    /// 这一条的不对称写进了签名：[`Coalition::band`] 答 `Result`（它问的是**本册自己的**
    /// 号空间，故有"查无此盟"），`bloc` 不答（它问的是**别人的**号空间，本册不去问）。
    pub fn bloc(&self, p: PrincipalId, after: Option<CoalitionId>) -> Window<CoalitionId> {
        self.window(|a| a.who == p, |a| a.of, after)
    }

    /// 一趟取窗：从 `book` 里挑相符的那些，按**号序**取「大于 `after` 的前 [`WINDOW_CAP`] 枚」。
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
                // **`put` 就是那一格判定**："塞得下"继续取，"塞不下"当场记下"还有"。
                Some(at) if out.put(at) => {
                    last = Some(at);
                }
                Some(_) => break,
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
