//! principal::core — **名册与谱系**：两张表、七条原语。
//!
//! 本文件**不 `use` 内核**：喂一串假 TID 就能把七条规矩推理干净。
//!
//! ```text
//!   名册   TID ──→ 当前 PolicyId      按内核盖的章查；可增可删；一 TID 一格
//!   谱系   PolicyId ──父──→ PolicyId   只增不删；下标即号；零号节点是根
//! ```
//!
//! 两条轴的分工见正文（[`super`]）：**名册**答"这个 Task 此刻代表谁"，**谱系**答
//! "这条身份从谁而来"。七条原语里只有三条动数据（`bind` / `unbind` / `derive`）。

use alloc::vec::Vec;

use env::TaskId;

// ── 号 ──────────────────────────────────────────────────────

/// 谱系上的一个节点。
///
/// **裸号**：与 [`TaskId`] 同形（8 字节、小端上线），但**不同源**——两个号空间互相拿错正是
/// 旧树栽过的那一格。故"这条号在不在树里"不是类型义务，是每条读操作查一次表答出来的
/// [`Fail::Unknown`]；[`PolicyId::new`] 造得出任何号，那正是探针验第三态的路子。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PolicyId(usize);

impl PolicyId {
    /// 根：Server 启动时自带的那一枚，**唯一没有父的节点**。
    pub const ROOT: PolicyId = PolicyId(0);

    /// 由裸号造一个（线上解码面；树外的号从这里进来）。
    pub const fn new(raw: usize) -> PolicyId {
        PolicyId(raw)
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
    pub const fn from_bytes(bytes: [u8; 8]) -> PolicyId {
        PolicyId(u64::from_le_bytes(bytes) as usize)
    }
}

// ── 失败域 ──────────────────────────────────────────────────

/// 失败域：三格，每格一个**不同的下一步**。
///
/// **`Resolve` 与三条谱系读没有失败域**——读是公开的（答案不是秘密，Principal 不授予任何
/// 东西）；这里三格只被写的那两条与"查无此节点"用。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    /// 你不是写名册的那一枚（`Bind`/`Unbind`），或不是"当前正好代表 `p`"的那一枚（`derive`）。
    /// 调用方接下来要改的是：**该请谁来做**。
    Denied,
    /// 这条 PolicyId 不在树里，或这个 TID 没绑过。调用方要改的是：**我手里这个号是假的**。
    Unknown,
    /// `try_reserve` 备不下。调用方要改的是：**晚点再来**。
    NoRoom,
}

// ── 两张表 ──────────────────────────────────────────────────

/// 树上一格：**只有父**。
///
/// 没有 setter ⇒ 父不可改；树只 `push` ⇒ 无环、恰好一个根（第二枚无父节点写不出来）。
struct Node {
    parent: Option<PolicyId>,
}

/// 名册一格：**一 TID 一格，且必有身份**（`policy` 不是 [`Option`]）。
///
/// 这里**没有"死活"这一列**——陈旧绑定必须合法：对方死了，收到它那句话的服务仍要问得出
/// "刚才是谁"。板/树那两处的惰性剔除在这里连挂点都没有（没有东西可剔）。
struct Bound {
    tid: TaskId,
    policy: PolicyId,
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

    /// 名册 · 写：把一条 TID 定到一条**已存在**的 PolicyId 上（只有装配者能写）。
    ///
    /// 覆盖 = **换绑**（不是 `Taken`）：同一位写第二次要么是更正，要么是装配单写错了，
    /// 都不是"别人抢了"——与 operator 否掉"名字已被占"同一条理由。
    pub fn bind(&mut self, from: TaskId, tid: TaskId, p: PolicyId) -> Result<(), Fail> {
        if from != self.assembler {
            return Err(Fail::Denied);
        }
        if self.node(p).is_none() {
            return Err(Fail::Unknown);
        }
        if let Some(row) = self.roster.iter_mut().find(|r| r.tid == tid) {
            row.policy = p;
            return Ok(());
        }
        self.roster.try_reserve(1).map_err(|_| Fail::NoRoom)?;
        self.roster.push(Bound { tid, policy: p });
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

    /// 名册 · 读：这条 TID 此刻代表谁。
    ///
    /// **没有失败域**：`None`（没绑）是一个诚实的答案，不是错误码——收到它的人自己决定
    /// 怎么对待一条没身份的请求。
    pub fn resolve(&self, tid: TaskId) -> Option<PolicyId> {
        self.roster.iter().find(|r| r.tid == tid).map(|r| r.policy)
    }

    // ── 谱系 ────────────────────────────────────────────────

    /// 谱系 · 写：由 `p` 派生一枚新节点，返新号。
    ///
    /// 钥匙两把，都是内核与名册免费给的：**装配者**，或**当前正好代表 `p` 的那一枚**
    /// （`resolve(from) == Some(p)`）。注意是"正好相等"，不是"`p` 的某个后代"——
    /// 于是"只能沿自己的 lineage 向下"从纪律变成判据，不需要新机制。
    pub fn derive(&mut self, from: TaskId, p: PolicyId) -> Result<PolicyId, Fail> {
        if from != self.assembler && self.resolve(from) != Some(p) {
            return Err(Fail::Denied);
        }
        if self.node(p).is_none() {
            return Err(Fail::Unknown);
        }
        self.tree.try_reserve(1).map_err(|_| Fail::NoRoom)?;
        let id = PolicyId(self.tree.len());
        self.tree.push(Node { parent: Some(p) });
        Ok(id)
    }

    /// 谱系 · 读：直接父。**三态**——`Ok(Some)` 有父 / `Ok(None)` 只有根 / `Err(Unknown)` 树外。
    ///
    /// 三格不许塌成一格：头注第 5 条（一格判据只问一件事）在这里的落点就是它。
    pub fn sire(&self, p: PolicyId) -> Result<Option<PolicyId>, Fail> {
        self.node(p).map(|n| n.parent).ok_or(Fail::Unknown)
    }

    /// 谱系 · 读：`a ≼ b`（`b` 在 `a` 那一支里，含 `a == b`）。
    pub fn heir(&self, a: PolicyId, b: PolicyId) -> Result<bool, Fail> {
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
    pub fn clan(&self, a: PolicyId, b: PolicyId) -> Result<PolicyId, Fail> {
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
        Ok(PolicyId::ROOT)
    }

    /// 树上一格（树外答 `None`）。
    fn node(&self, p: PolicyId) -> Option<&Node> {
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
        assert_eq!(b.sire(PolicyId::ROOT), Ok(None));
        assert_eq!(b.sire(PolicyId::new(4095)), Err(Fail::Unknown));
    }

    #[test]
    fn a_bound_task_resolves_and_rebinding_replaces() {
        let mut b = book();
        let p = b.derive(A, PolicyId::ROOT).expect("装配者能派生");
        assert_eq!(b.resolve(ME), None);
        b.bind(A, ME, p).expect("装配者能绑");
        assert_eq!(b.resolve(ME), Some(p));
        let q = b.derive(A, PolicyId::ROOT).unwrap();
        b.bind(A, ME, q).unwrap();
        assert_eq!(b.resolve(ME), Some(q));
    }

    #[test]
    fn only_the_assembler_writes_the_roster() {
        let mut b = book();
        let p = b.derive(A, PolicyId::ROOT).unwrap();
        assert_eq!(b.bind(ME, ME, p), Err(Fail::Denied));
        assert_eq!(b.unbind(ME, ME), Err(Fail::Denied));
        assert_eq!(b.unbind(A, ME), Err(Fail::Unknown));
    }

    #[test]
    fn a_representative_derives_downwards_only() {
        let mut b = book();
        let p = b.derive(A, PolicyId::ROOT).unwrap();
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
        let p = b.derive(A, PolicyId::ROOT).unwrap();
        let q = b.derive(A, p).unwrap();
        assert_eq!(b.heir(p, p), Ok(true));
        assert_eq!(b.heir(p, q), Ok(true));
        assert_eq!(b.heir(q, p), Ok(false));
        assert_eq!(b.heir(PolicyId::new(4095), p), Err(Fail::Unknown));
    }

    #[test]
    fn clan_meets_at_the_nearest_common_ancestor() {
        let mut b = book();
        let x = b.derive(A, PolicyId::ROOT).unwrap();
        let y = b.derive(A, PolicyId::ROOT).unwrap();
        let z = b.derive(A, y).unwrap();
        assert_eq!(b.clan(x, z), Ok(PolicyId::ROOT));
        assert_eq!(b.clan(y, z), Ok(y));
        assert_eq!(b.clan(z, z), Ok(z));
    }
}
