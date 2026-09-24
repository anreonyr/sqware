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
    /// **两条转换原语到不了这一格**（它们不分配）：到得了的是 `new`（立根）、`bind`、`derive`。
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

    /// 名册 · 写：撤一格（只有装配者能写）。**线上不发**——核心有、报文里没有这一格。
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
    /// （核心有、线上不发）。
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

// ── 用例不在这里（照实记：用户裁定"测试和运行环境分开"）──────────────
//
// 本文件原先那个 `#[cfg(test)] mod tests`（**7 条**）整体搬去了 `crates/protocol-case` 的
// `roster` 靶里 `principal_core` 那一格，门口 `crates/gate/tests/host.rs`；**本文件从此没有一行测试**。
//
// 那一批原先是"编不到、也跑不到"的规格（`protocol` 是 `[lib] test = false`，riscv 上编不出
// libtest）；它们真正被跑起来，是从那台宿主靶开始。真机上另有探针那几条
// （`harness/src/subject.rs`）。
