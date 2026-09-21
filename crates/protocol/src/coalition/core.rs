//! coalition::core — **盟册**：一条关系、一枚计数器、六条原语。
//!
//! 本文件**不 `use` 内核**——连 `TaskId` 都不认识（喂两个假号就能把六条规矩推理干净）。
//! 这不是风格：身份那一侧归 [`principal`](crate::principal)（名册把内核盖的章翻成一条号），
//! 本册只收**一条已经解析好的身份**；故"这条请求是不是发送者本人"不在这一层——它在适配层
//! （见正文"已知边界"）。
//!
//! ```text
//!   盟籍   PolicyId ──→ CoalitionId     这条身份在哪些盟里     bloc(p)   核心
//!          CoalitionId ──→ PolicyId     这枚盟里有谁           band(c)   核心
//!   号     0 .. next                    铸过就一直在（没有墓碑、没有 id 表）
//! ```
//!
//! 两条轴的分工见正文（[`super`]）：**横向的盟籍**只有一张表，反着念就是第二个方向。
//! 六条原语里只有三条动数据（`found` / `enter` / `leave`）。

use alloc::vec::Vec;

use crate::principal::core::PolicyId;

// ── 号 ──────────────────────────────────────────────────────

/// 盟册上的一枚号。
///
/// **裸号**：与 [`PolicyId`] 同形（8 字节、小端上线），但**不同源**——两个号空间互相拿错正是
/// 旧树栽过的那一格。故"这枚号铸过没有"不是类型义务，是每条读查一次计数答出来的
/// [`Fail::Unknown`]；[`CoalitionId::new`] 造得出任何号，那正是探针验第三态的路子。
///
/// **没有 `ROOT`**：盟无根、无主——零号是一枚**普通的盟**（对照 [`PolicyId::ROOT`]：
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

// ── 一格盟籍 ────────────────────────────────────────────────

/// 盟籍一格：**一对号，没有第三格**。
///
/// 没有角色、没有权重、没有序位——"盟只是一组身份"这句话就落在这里：**类型里写不出别的**。
/// 唯一性（同一对不许出现两次）类型表达不了，落在 [`Coalition::enter`] 的**先查后推**上
/// （与 `Principal::bind` 覆盖那一趟同一形状）：**一次线性扫，换掉"两张表要同步"那条义务**。
struct Ally {
    who: PolicyId,
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
    pub fn enter(&mut self, who: PolicyId, c: CoalitionId) -> Result<(), Fail> {
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
    pub fn leave(&mut self, who: PolicyId, c: CoalitionId) -> Result<(), Fail> {
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
    pub fn amid(&self, p: PolicyId, c: CoalitionId) -> Result<bool, Fail> {
        if !self.stands(c) {
            return Err(Fail::Unknown);
        }
        Ok(self.book.iter().any(|a| a.who == p && a.of == c))
    }

    /// 盟 · 读：`c` 里此刻有谁。**核心，不上线**——答案是一串号，本仓还没有那种帧形
    /// （同 `operator::list` / `Principal::clan` 那一档：核心有、线上不发）。
    ///
    /// 返**惰性迭代器**（借 `&self`，不缓冲、不分配 ⇒ 到不了 [`Fail::NoRoom`]）；顺序是
    /// 登记序，**不是承诺**。
    pub fn band(&self, c: CoalitionId) -> Result<impl Iterator<Item = PolicyId> + '_, Fail> {
        if !self.stands(c) {
            return Err(Fail::Unknown);
        }
        Ok(self.book.iter().filter(move |a| a.of == c).map(|a| a.who))
    }

    /// 盟 · 读：`p` 此刻在哪些盟里。**没有失败域**——`p` 是标签，不在任何盟里就是空串。
    ///
    /// 这一条的不对称写进了签名：[`Coalition::band`] 答 `Result`（它问的是**本册自己的**
    /// 号空间，故有"查无此盟"），`bloc` 不答（它问的是**别人的**号空间，本册不去问）。
    pub fn bloc(&self, p: PolicyId) -> impl Iterator<Item = CoalitionId> + '_ {
        self.book.iter().filter(move |a| a.who == p).map(|a| a.of)
    }

    /// 这枚号铸过没有。**只有这一处读 `next`**——存在性的判据只此一份。
    fn stands(&self, c: CoalitionId) -> bool {
        c.get() < self.next
    }
}

#[cfg(test)]
mod tests {
    //! **照实记：本模块编不到，也跑不到**——`protocol/Cargo.toml` 是 `test = false`（riscv 目标上
    //! 编不出 libtest）。故下面这几条今天只是**契约的读数**，不是门；真机上跑的是探针那几条
    //! （`programs/src/user/member.rs`）。留在这里的理由与 `principal::core` 那几条同款：
    //! 换载体时照着它们走。
    use super::*;

    fn book() -> Coalition {
        Coalition::new()
    }

    const A: PolicyId = PolicyId::new(11);
    const B: PolicyId = PolicyId::new(22);
    /// 伪造的线上值：铸过的号是 `0..next`，故这个一定在册外。
    const OUTSIDE: CoalitionId = CoalitionId::new(4095);

    #[test]
    fn found_mints_dense_monotone_numbers_and_never_fails() {
        let mut b = book();
        assert_eq!(b.found(), CoalitionId::new(0));
        assert_eq!(b.found(), CoalitionId::new(1));
        // 空盟合法：铸出来一枚都没进，也没有"散掉"这回事。
        assert_eq!(b.amid(A, CoalitionId::new(0)), Ok(false));
    }

    #[test]
    fn a_coalition_outside_the_counter_is_unknown_for_writes_and_band() {
        let mut b = book();
        assert_eq!(b.enter(A, OUTSIDE), Err(Fail::Unknown));
        assert_eq!(b.leave(A, OUTSIDE), Err(Fail::Unknown));
        assert_eq!(b.amid(A, OUTSIDE), Err(Fail::Unknown));
        assert!(b.band(OUTSIDE).is_err());
    }

    #[test]
    fn enter_and_leave_are_idempotent_and_bloc_has_no_failure() {
        let mut b = book();
        let c = b.found();
        assert_eq!(b.enter(A, c), Ok(()));
        assert_eq!(b.enter(A, c), Ok(())); // 第二次：表不动
        assert_eq!(b.amid(A, c), Ok(true));
        assert_eq!(b.leave(A, c), Ok(()));
        assert_eq!(b.leave(A, c), Ok(())); // 撞空也成
        assert_eq!(b.amid(A, c), Ok(false));
        // 假身份：不在任何盟里 ⇒ 空串（**不是** Unknown——本册不去问身份服务）。
        assert_eq!(b.bloc(PolicyId::new(4095)).count(), 0);
    }

    #[test]
    fn one_a_pair_is_one_row_and_two_identities_can_share_a_coalition() {
        let mut b = book();
        let c0 = b.found();
        let c1 = b.found();
        b.enter(A, c0).unwrap();
        b.enter(B, c0).unwrap();
        b.enter(A, c1).unwrap();
        assert_eq!(b.band(c0).unwrap().collect::<Vec<_>>(), alloc::vec![A, B]);
        assert_eq!(b.band(c1).unwrap().collect::<Vec<_>>(), alloc::vec![A]);
        assert_eq!(b.bloc(A).collect::<Vec<_>>(), alloc::vec![c0, c1]);
        // 出去的是"这一对"，不是"这个人"。
        b.leave(A, c0).unwrap();
        assert_eq!(b.band(c0).unwrap().collect::<Vec<_>>(), alloc::vec![B]);
        assert_eq!(b.bloc(A).collect::<Vec<_>>(), alloc::vec![c1]);
    }
}
