//! principal::core — **名册与谱系**：两张表、九条原语。
//!
//! 本文件**不 `use` 内核**：喂一串假 TID 就能把九条规矩推理干净。
//!
//! ```text
//!   名册   TID ──→ 当前 PrincipalId      按内核盖的章查；可增可删；一 TID 一格
//!   谱系   PrincipalId ──父──→ PrincipalId   只增不删；下标即号；零号节点是根
//! ```
//!
//! 两条轴的分工见正文（[`super`]）：**名册**答"这个 Task 此刻代表谁"，**谱系**答
//! "这条身份从谁而来"。九条原语里只有**五条**动数据（`bind` / `unbind` / `derive` /
//! `adopt` / `waive`）——其中前三条动**结构**（行与节点），后两条只改写 `current`。
//! 照实记：这一句原写"七条原语里只有三条动数据"，那是转换那两条还没落地时的口径。

use alloc::vec::Vec;

use env::TaskId;

// ── 号 ──────────────────────────────────────────────────────

/// 谱系上的一个节点。
///
/// **裸号**：与 [`TaskId`] 同形（8 字节、小端上线），但**不同源**——两个号空间互相拿错正是
/// 旧树栽过的那一格。故"这条号在不在树里"不是类型义务，是每条读操作查一次表答出来的
/// [`Fail::Unknown`]；[`PrincipalId::new`] 造得出任何号，那正是探针验第三态的路子。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PrincipalId(usize);

impl PrincipalId {
    /// 根：Server 启动时自带的那一枚，**唯一没有父的节点**。
    pub const ROOT: PrincipalId = PrincipalId(0);

    /// 由裸号造一个（线上解码面；树外的号从这里进来）。
    pub const fn new(raw: usize) -> PrincipalId {
        PrincipalId(raw)
    }

    /// 裸号。
    pub const fn get(self) -> usize {
        self.0
    }

    /// 线上的那一格（8 字节小端）。
    pub const fn to_bytes(self) -> [u8; 8] {
        (self.0 as u64).to_le_bytes()
    }

    /// 由线上字节还原（**不校验**：在不在树里由核心答）。
    pub const fn from_bytes(bytes: [u8; 8]) -> PrincipalId {
        PrincipalId(u64::from_le_bytes(bytes) as usize)
    }
}

// ── 失败域 ──────────────────────────────────────────────────

/// 失败域：三格，每格一个**不同的下一步**。
///
/// **`Resolve` 与三条谱系读没有失败域**——读是公开的（答案不是秘密，Principal 不授予任何
/// 东西）；这里三格只被写的那两条与"查无此节点"用。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    /// 你不是那一个：不是写名册的那一枚（`Bind`/`Unbind`）、不是"当前正好代表 `p`"的那一枚
    /// （`derive`）、或目标不在**你自己那一支**里（`adopt`）。调用方要改的是：**该请谁来做**
    /// 或**换一个目标**。
    Denied,
    /// 这条 PrincipalId 不在树里，或这个 TID 没绑过。调用方要改的是：**我手里这个号是假的**。
    Unknown,
    /// `try_reserve` 备不下。调用方要改的是：**晚点再来**。
    ///
    /// **两条转换原语到不了这一格**（它们不分配）：这一格今天只被 `bind` / `derive` 用。
    NoRoom,
}

// ── 两张表 ──────────────────────────────────────────────────

/// 树上一格：**只有父**。
///
/// 没有 setter ⇒ 父不可改；树只 `push` ⇒ 无环、恰好一个根（第二枚无父节点写不出来）。
struct Node {
    parent: Option<PrincipalId>,
}

/// 名册一格：**一 TID 一格，且必有起点、必有当前**（两格都不是 [`Option`]）。
///
/// - `origin` = 装配者当初把它定在哪条号上；`waive` 回到这里，于是"放弃"不是一个无底洞；
/// - `current` = 此刻代表哪条号；`adopt` 只改这一格。
///
/// 不变量 **`origin ≼ current` 恒成立**：`bind` 两格同写起手、`adopt` 沿支走一步、
/// `waive` 把 `current` 写回 `origin`——归纳成立。一句话说清两条转换原语的全部语义：
/// **身份只会沿自己那一支往下走，或者回到起点。**
///
/// 这里**没有"死活"这一列**——陈旧绑定必须合法：对方死了，收到它那句话的服务仍要问得出
/// "刚才是谁"。板/树那两处的惰性剔除在这里连挂点都没有（没有东西可剔）。
struct Bound {
    tid: TaskId,
    origin: PrincipalId,
    current: PrincipalId,
}

/// 身份服务的权威状态：**两张表 + 一把钥匙**。
pub struct Principal {
    /// 唯一能写名册的那一枚——Server 启动时把它自己的 `Sire` 注入进来。
    assembler: TaskId,
    /// 谱系：只增不删，下标即号。
    tree: Vec<Node>,
    /// 名册：一 TID 一格，可增可删。
    roster: Vec<Bound>,
}

impl Principal {
    /// 立一份账：**自带根**（零号节点，没有父）。
    ///
    /// 根立不起来就没有身份可言，故备不下时如实报 [`Fail::NoRoom`]（不 panic）。
    pub fn new(assembler: TaskId) -> Result<Principal, Fail> {
        let mut tree = Vec::new();
        tree.try_reserve(1).map_err(|_| Fail::NoRoom)?;
        tree.push(Node { parent: None });
        Ok(Principal {
            assembler,
            tree,
            roster: Vec::new(),
        })
    }

    // ── 名册 ────────────────────────────────────────────────

    /// 名册 · 写：把一条 TID 定到一条**已存在**的 PrincipalId 上（只有装配者能写）。
    ///
    /// 覆盖 = **换绑**（不是 `Taken`）：同一位写第二次要么是更正，要么是装配单写错了，
    /// 都不是"别人抢了"——与 operator 否掉"名字已被占"同一条理由。
    ///
    /// **换绑 = 重定起点**：两格一起写（`origin` 与 `current` 同值）。
    pub fn bind(&mut self, from: TaskId, tid: TaskId, p: PrincipalId) -> Result<(), Fail> {
        if from != self.assembler {
            return Err(Fail::Denied);
        }
        if self.node(p).is_none() {
            return Err(Fail::Unknown);
        }
        if let Some(row) = self.roster.iter_mut().find(|r| r.tid == tid) {
            row.origin = p;
            row.current = p;
            return Ok(());
        }
        self.roster.try_reserve(1).map_err(|_| Fail::NoRoom)?;
        self.roster.push(Bound {
            tid,
            origin: p,
            current: p,
        });
        Ok(())
    }

    /// 名册 · 写：撤一格（只有装配者能写）。**线上不发**，同 `board::unregister` 那一档。
    ///
    /// 撞空答 [`Fail::Unknown`]（不是幂等的"收到"）：装配者是自己那本账的主人，
    /// "这一格本来就没有"是一条值得如实回答的事实。
    pub fn unbind(&mut self, from: TaskId, tid: TaskId) -> Result<(), Fail> {
        if from != self.assembler {
            return Err(Fail::Denied);
        }
        let at = self
            .roster
            .iter()
            .position(|r| r.tid == tid)
            .ok_or(Fail::Unknown)?;
        self.roster.remove(at);
        Ok(())
    }

    /// 名册 · 读：这条 TID **此刻**代表谁。
    ///
    /// **没有失败域**：`None`（没绑）是一个诚实的答案，不是错误码——收到它的人自己决定
    /// 怎么对待一条没身份的请求。读的是 `current`（`adopt` 改过的那一格）。
    pub fn resolve(&self, tid: TaskId) -> Option<PrincipalId> {
        self.roster.iter().find(|r| r.tid == tid).map(|r| r.current)
    }

    // ── 谱系 ────────────────────────────────────────────────

    /// 谱系 · 写：由 `p` 派生一枚新节点，返新号。
    ///
    /// 钥匙两把，都是内核与名册免费给的：**装配者**，或**当前正好代表 `p` 的那一枚**
    /// （`resolve(from) == Some(p)`）。注意是"正好相等"，不是"`p` 的某个后代"——
    /// 于是"只能沿自己的 lineage 向下"从纪律变成判据，不需要新机制。
    pub fn derive(&mut self, from: TaskId, p: PrincipalId) -> Result<PrincipalId, Fail> {
        if from != self.assembler && self.resolve(from) != Some(p) {
            return Err(Fail::Denied);
        }
        if self.node(p).is_none() {
            return Err(Fail::Unknown);
        }
        self.tree.try_reserve(1).map_err(|_| Fail::NoRoom)?;
        let id = PrincipalId(self.tree.len());
        self.tree.push(Node { parent: Some(p) });
        Ok(id)
    }

    /// 谱系 · 读：直接父。**三态**——`Ok(Some)` 有父 / `Ok(None)` 只有根 / `Err(Unknown)` 树外。
    ///
    /// 三格不许塌成一格：头注第 5 条（一格判据只问一件事）在这里的落点就是它。
    pub fn sire(&self, p: PrincipalId) -> Result<Option<PrincipalId>, Fail> {
        self.node(p).map(|n| n.parent).ok_or(Fail::Unknown)
    }

    /// 谱系 · 读：`a ≼ b`（`b` 在 `a` 那一支里，含 `a == b`）。
    pub fn heir(&self, a: PrincipalId, b: PrincipalId) -> Result<bool, Fail> {
        if self.node(a).is_none() || self.node(b).is_none() {
            return Err(Fail::Unknown);
        }
        let mut at = Some(b);
        while let Some(cur) = at {
            if cur == a {
                return Ok(true);
            }
            at = self.node(cur).and_then(|n| n.parent);
        }
        Ok(false)
    }

    /// 谱系 · 读：最近公共祖先（`clan`）。**核心，不上线**——今天没有真客人
    /// （同 `operator::list` 那一档：核心有、线上不发）。
    ///
    /// 单根 ⇒ 必有解（最坏是根）；`a == b` 时答它自己。
    pub fn clan(&self, a: PrincipalId, b: PrincipalId) -> Result<PrincipalId, Fail> {
        if self.node(a).is_none() || self.node(b).is_none() {
            return Err(Fail::Unknown);
        }
        let mut at = Some(a);
        while let Some(cur) = at {
            if self.heir(cur, b)? {
                return Ok(cur);
            }
            at = self.node(cur).and_then(|n| n.parent);
        }
        // 到不了：根是所有人的祖先。
        Ok(PrincipalId::ROOT)
    }

    // ── 转换 ────────────────────────────────────────────────

    /// 转换 · 领：把**自己**当前的号换成 `q`。
    ///
    /// 三格前置各自不同一件事：
    /// - `from` 那一格在名册里，取 `p = current`；否则 [`Fail::Unknown`]——与 [`Principal::unbind`]
    ///   撞空同一格："那一格不存在"，调用方接下来都是"先让装配者给我绑"；
    /// - `q` 在树里；否则 [`Fail::Unknown`]（这个号是假的）；
    /// - **`heir(p, q)`**；否则 [`Fail::Denied`]——向上、跨支都不在**你自己那一支**里。
    ///
    /// `q == p` 时幂等。不动树、不动 `origin`（故 `origin ≼ current` 仍成立）。
    pub fn adopt(&mut self, from: TaskId, q: PrincipalId) -> Result<(), Fail> {
        // 先把三格判据问完（都是读），再去动那一格——借还清楚了，规矩也一眼看得见。
        let Some(p) = self.resolve(from) else {
            return Err(Fail::Unknown);
        };
        if self.node(q).is_none() {
            return Err(Fail::Unknown);
        }
        if !self.heir(p, q)? {
            return Err(Fail::Denied);
        }
        let row = self
            .roster
            .iter_mut()
            .find(|r| r.tid == from)
            .ok_or(Fail::Unknown)?;
        row.current = q;
        Ok(())
    }

    /// 转换 · 弃：把 `current` 写回 `origin`——**回到装配给我的那一条**。
    ///
    /// 认的是"发送者是谁"这一格（内核盖章），没有参数也没有别的钥匙：那一格在名册里就成。
    /// 撞空（没绑过）答 [`Fail::Unknown`]，与 [`Principal::unbind`] 同调。
    ///
    /// **不删格**：身份还在，只是回到起点——故"放弃"不是单向门（再 `adopt` 一次就回去了）。
    /// 本来就相等时幂等。
    pub fn waive(&mut self, from: TaskId) -> Result<(), Fail> {
        let Some(row) = self.roster.iter_mut().find(|r| r.tid == from) else {
            return Err(Fail::Unknown);
        };
        row.current = row.origin;
        Ok(())
    }

    /// 树上一格（树外答 `None`）。
    fn node(&self, p: PrincipalId) -> Option<&Node> {
        self.tree.get(p.get())
    }
}

#[cfg(test)]
mod tests {
    //! **照实记：本模块编不到，也跑不到**——`protocol/Cargo.toml` 是 `test = false`（riscv 目标上
    //! 编不出 libtest）。故下面这几条今天只是**契约的读数**，不是门；真机上跑的是探针那六条
    //! （`programs/src/user/subject.rs`）。留在这里的理由与 `system::board::core` 那六条同款：
    //! 换载体时照着它们走。
    use super::*;

    const A: TaskId = TaskId::new(11); // 装配者
    const ME: TaskId = TaskId::new(22); // 一条被别人绑的 TID

    fn book() -> Principal {
        Principal::new(A).expect("根立得起来")
    }

    #[test]
    fn the_root_has_no_sire_and_a_tree_outside_id_is_unknown() {
        let b = book();
        assert_eq!(b.sire(PrincipalId::ROOT), Ok(None));
        assert_eq!(b.sire(PrincipalId::new(4095)), Err(Fail::Unknown));
    }

    #[test]
    fn a_bound_task_resolves_and_rebinding_replaces() {
        let mut b = book();
        let p = b.derive(A, PrincipalId::ROOT).expect("装配者能派生");
        assert_eq!(b.resolve(ME), None);
        b.bind(A, ME, p).expect("装配者能绑");
        assert_eq!(b.resolve(ME), Some(p));
        let q = b.derive(A, PrincipalId::ROOT).unwrap();
        b.bind(A, ME, q).unwrap();
        assert_eq!(b.resolve(ME), Some(q));
    }

    #[test]
    fn only_the_assembler_writes_the_roster() {
        let mut b = book();
        let p = b.derive(A, PrincipalId::ROOT).unwrap();
        assert_eq!(b.bind(ME, ME, p), Err(Fail::Denied));
        assert_eq!(b.unbind(ME, ME), Err(Fail::Denied));
        assert_eq!(b.unbind(A, ME), Err(Fail::Unknown));
    }

    #[test]
    fn a_representative_derives_downwards_only() {
        let mut b = book();
        let p = b.derive(A, PrincipalId::ROOT).unwrap();
        b.bind(A, ME, p).unwrap();
        let q = b.derive(ME, p).expect("代表 p 的那一枚能向下派生");
        assert_eq!(b.sire(q), Ok(Some(p)));
        // 换绑到 q 之后，同一枚不能再回头派生 p 的另一个孩子（钥匙是"正好代表 p"）。
        b.bind(A, ME, q).unwrap();
        assert_eq!(b.derive(ME, p), Err(Fail::Denied));
    }

    #[test]
    fn heir_is_reflexive_asymmetric_and_three_state() {
        let mut b = book();
        let p = b.derive(A, PrincipalId::ROOT).unwrap();
        let q = b.derive(A, p).unwrap();
        assert_eq!(b.heir(p, p), Ok(true));
        assert_eq!(b.heir(p, q), Ok(true));
        assert_eq!(b.heir(q, p), Ok(false));
        assert_eq!(b.heir(PrincipalId::new(4095), p), Err(Fail::Unknown));
    }

    #[test]
    fn clan_meets_at_the_nearest_common_ancestor() {
        let mut b = book();
        let x = b.derive(A, PrincipalId::ROOT).unwrap();
        let y = b.derive(A, PrincipalId::ROOT).unwrap();
        let z = b.derive(A, y).unwrap();
        assert_eq!(b.clan(x, z), Ok(PrincipalId::ROOT));
        assert_eq!(b.clan(y, z), Ok(y));
        assert_eq!(b.clan(z, z), Ok(z));
    }

    #[test]
    fn conversion_keeps_to_its_own_branch_and_waive_returns_to_origin() {
        let mut b = book();
        // 装配给 ME 的起点是 p；p 下再派生 q；另有一支 r 不在 p 下面。
        let p = b.derive(A, PrincipalId::ROOT).unwrap();
        b.bind(A, ME, p).unwrap();
        let q = b.derive(A, p).unwrap();
        let r = b.derive(A, PrincipalId::ROOT).unwrap();

        assert_eq!(b.adopt(ME, q), Ok(()));
        assert_eq!(b.resolve(ME), Some(q));
        // 不变量：起点始终是自己当前那一格的祖先。
        assert_eq!(b.heir(b.roster[0].origin, b.roster[0].current), Ok(true));
        // **钥匙反证**：已不代表 p，故"从 p 派生"被拒——转换是真的。
        assert_eq!(b.derive(ME, p), Err(Fail::Denied));
        // 向上（p 是 q 的父，不在 q 那一支里）与跨支都不许。
        assert_eq!(b.adopt(ME, p), Err(Fail::Denied));
        assert_eq!(b.adopt(ME, r), Err(Fail::Denied));
        // 树外。
        assert_eq!(b.adopt(ME, PrincipalId::new(4095)), Err(Fail::Unknown));
        // 弃 = 回到起点；**不删格**，故还能再领一次。
        assert_eq!(b.waive(ME), Ok(()));
        assert_eq!(b.resolve(ME), Some(p));
        assert_eq!(b.adopt(ME, q), Ok(()));
        // 换绑 = 重定起点：弃回的是**新**起点（两格一起写）。
        b.bind(A, ME, r).unwrap();
        assert_eq!(b.resolve(ME), Some(r));
        assert_eq!(b.waive(ME), Ok(()));
        assert_eq!(b.resolve(ME), Some(r));

        // 没绑过的那一枚：两条转换都答 Unknown（与 unbind 撞空同调）。
        let other = TaskId::new(33);
        assert_eq!(b.adopt(other, p), Err(Fail::Unknown));
        assert_eq!(b.waive(other), Err(Fail::Unknown));
    }
}
