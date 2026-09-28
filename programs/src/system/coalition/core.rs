//! coalition::core — **盟册那一本账**：一条关系 ＋ 一条主子 ＋ 一枚计数器。
//!
//! **照实记（它原先住 `protocol::system::coalition::core`）**：那一份的读者只有本域的持有者
//! （`prog-coalition` 那一枚线程）——按"协议 = 共享语言"的判据（"只有实现方读得到它"），
//! 它属于实现侧。协议那一侧只留**上线的类型**：号（`CoalitionId`）、失败域（`Fail`）、
//! 一窗号（`Window` / `WINDOW_CAP`）。
//!
//! 本文件只讲账的形状与那七条原语；**盟籍的含义、钥匙那一格、帧那一层**都写在协议那一边
//! （`protocol::system::coalition`）。
//!
//! # 照实记（"盟无主"那条裁定**翻案**了）
//!
//! 这一族原先的裁定是 **K2：盟无主**——立盟那位不留名，故"谁都不比谁大"，连 `found` 都
//! **不分配**（一枚计数器就是全部）。翻案翻的是**这一条**：设备账那一台要**替**四位驱动报名
//! （`hub::bond` ⇒ `admit`），而入盟那把钥匙是**发送者那一格**（`Coalition::enter` 只表达
//! "**我**进"）⇒ 没有"谁立的盟"，"许某一位进这一类"这句话在模型里**说不出来**。
//!
//! **翻的是哪一格、没翻哪一格**：盟籍仍旧**只是一对号**（[`Ally`]：没有角色、没有权重、
//! 没有序位）；多出来的是**一格盟主**（[`Chief`]，立盟那位），而它只答一句话——"代报名
//! 这件事归不归你"。`enter` 那条"诚实性由签名给"的口径照旧成立。

use alloc::vec::Vec;

use protocol::id::Id;
use protocol::system::coalition::{CoalitionId, Fail, Window};
use protocol::system::principal::PrincipalId;

// ── 一格盟籍 ────────────────────────────────────────────────

/// 盟籍一格：**一对号，没有第三格**（K2 没翻的那一半）。
///
/// 没有角色、没有权重、没有序位——"盟只是一组身份"这句话就落在这里：**类型里写不出别的**。
/// 唯一性（同一对不许出现两次）类型表达不了，落在 [`Coalition::enter`] 的**先查后推**上
/// （与 `Principal::bind` 覆盖那一趟同一形状）：**一次线性扫，换掉"两张表要同步"那条义务**。
///
/// **盟主不在这一格里**（照实记）：它是**一枚盟**的属性，不是**一条盟籍**的属性——
/// 写进这里就会得出"盟主也是一名成员"（而那位可能压根不在自己的盟里）。
struct Ally {
    who: PrincipalId,
    of: CoalitionId,
}

/// **一格盟主**：这一枚盟是谁立的（K2 翻案添的那一格）。
///
/// 它只有一个读者——[`Coalition::admit`] 那一句"代报名这件事归不归你"。
struct Chief {
    of: CoalitionId,
    chief: PrincipalId,
}

// ── 状态 ────────────────────────────────────────────────────

/// 盟册的权威状态：**一条关系 ＋ 一条主子 ＋ 一枚计数器**。
///
/// **照实记（`assembler` 那一格还是没有，但理由变了）**：对照 `Principal.assembler`
/// （那是名册的钥匙），这里仍然没有"钥匙那一格"——盟主**不是钥匙**，它只许一件事
/// （[`Coalition::admit`]），别的六条原语一个都不看它。
pub struct Coalition {
    /// 号的存在那一格：**号 < `next` 即铸过**。只增（全文件没有一处减它）。
    ///
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
    ///
    /// 不分配 ⇒ 不失败（对照 `Principal::new`：它要为根 `try_reserve`）。
    pub const fn new() -> Coalition {
        Coalition {
            next: 0,
            book: Vec::new(),
            chiefs: Vec::new(),
        }
    }

    // ── 立 ──────────────────────────────────────────────────

    /// 盟 · 写：立一枚新号，并记下**立它那一位**（盟主）。号取当前计数，再把计数加一。
    ///
    /// **失败域从"没有"变成一格**（照实记：K2 翻案那一刀）：这一手原先**不分配** ⇒ 全族唯一
    /// 一条不可失败的原语；今天它要多记一行盟主 ⇒ [`Fail::Full`] 到得了这一格。**次序是契约**：
    /// 先备下那一行、再动号——备不下就**一枚号都不铸**（不留一枚没有主的盟，因为那种盟
    /// 谁都不许代报名，而它照样占着号）。
    ///
    /// 钥匙（发送者得是个已绑定的身份）在**适配层**：核心不认识 TID，也不持名册，收的是
    /// **一条已经解析好的身份**。
    ///
    /// 号因此仍单调、稠密、"铸过就一直在"；空盟合法且免费（没有墓碑，也没有 id 表）。
    pub fn found(&mut self, who: PrincipalId) -> Result<CoalitionId, Fail> {
        self.chiefs.try_reserve(1).map_err(|_| Fail::Full)?;
        let id = CoalitionId::new(self.next);
        self.next += 1;
        self.chiefs.push(Chief { of: id, chief: who });
        Ok(id)
    }

    // ── 入 / 出 ─────────────────────────────────────────────

    /// 盟 · 写：把 `who` 放进 `c`。**幂等**：已在里面答 `Ok(())`、表不动——集合没有"第二次"。
    ///
    /// 两格前置各问一件事：`c` 铸过没有（[`Fail::Unknown`]）、备得下那一行吗（[`Fail::Full`]）。
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
        self.book.try_reserve(1).map_err(|_| Fail::Full)?;
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

    // ── 代报名 ──────────────────────────────────────────────

    /// 盟 · 写：把 `target` 放进 `c`——**只有这一枚盟的盟主叫得动**。
    ///
    /// 这就是"替别人入盟"唯一那条路（K2 翻案那一刀，见本文件头注）：`enter` 的钥匙是
    /// **发送者那一格**（只表达得了"我进"），故"许某一位进这一类"只能由立盟那位说。
    ///
    /// 三格前置：`c` 铸过没有（[`Fail::Unknown`]）、**你是不是这一枚的盟主**
    /// （[`Fail::NotChief`]）、备得下那一行吗（[`Fail::Full`]，由 [`Coalition::enter`] 答）。
    ///
    /// **`chief` 由适配层解**（同 `found`：核心收的是已经解析好的身份）；**`target` 也是**
    /// ——帧里那一格是 TID，名是服务端过名册点的。
    ///
    /// **照实记（读不出来的那一格往"拒"的一侧落）**：一枚盟读不出盟主（构造上到不了：
    /// `found` 要么记上主、要么不铸这一枚号）⇒ 答 [`Fail::NotChief`]，**不放行**。
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

