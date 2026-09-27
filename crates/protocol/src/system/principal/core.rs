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

use crate::id::Id;

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
    ///
    /// **照实记（它今天没有代码读者，留着是有意的）**：`clan` 是它唯一的使用者，那一具已按
    /// 残枝删掉 ⇒ 这一格只剩"号空间的事实"这一重身份。**不删**：盟册与树那一侧有六处正文
    /// 拿它当锚（"零号是真格子"——`CoalitionId(0)` / `EntryId` 的对照都指着这里），删了那些
    /// 说法就没有落点。它不是机制，是一个**被引用的事实常量**。
    pub const ROOT: PrincipalId = PrincipalId(0);

    /// 由裸号造一个（线上解码面；树外的号从这里进来）。
    pub const fn new(raw: usize) -> PrincipalId {
        PrincipalId(raw)
    }

    /// 裸号。
    pub const fn get(self) -> usize {
        self.0
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
    Full,
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
    /// 根立不起来就没有身份可言，故备不下时如实报 [`Fail::Full`]（不 panic）。
    pub fn new(assembler: TaskId) -> Result<Principal, Fail> {
        let mut tree = Vec::new();
        tree.try_reserve(1).map_err(|_| Fail::Full)?;
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
    /// 覆盖 = **换绑**（不是 `Taken`）：同一位写第二次要么是更正，要么是装配表写错了，
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
        self.roster.try_reserve(1).map_err(|_| Fail::Full)?;
        self.roster.push(Bound {
            tid,
            origin: p,
            current: p,
        });
        Ok(())
    }

    // **照实记（`unbind` 已删）**：名册那一侧原先还有一具"撤一格"的写（`unbind(from, tid)`），
    // 只有装配者叫得动、且**线上不发**。全仓零调用者（`pcall::Wire` 里也没有这一格）⇒ 按
    // "没有读者的格不留在面上"删掉。名册的写从此只有 [`Principal::bind`] 与 [`Principal::adopt`]。

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
        self.tree.try_reserve(1).map_err(|_| Fail::Full)?;
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

    // **照实记（`clan` 已删）**：谱系那一侧原先还有一具"最近公共祖先"的读（`clan(a, b)`），
    // 头注自己写着"核心，不上线——今天没有真客人"。全仓零调用者 ⇒ 按"没有读者的格不留在
    // 面上"删掉。要它的时候，`heir` 已经够（本函数的身体就是"自 `a` 上溯，第一次在 `b` 那一支里"）。

    // ── 转换 ────────────────────────────────────────────────

    /// 转换 · 领：把**自己**当前的号换成 `q`。
    ///
    /// **同名反义**：内核那一侧的 `Task::adopt(child)` 是把一枚子域**收进来**，与这里"把**自己**
    /// 换到下面"方向正好相反（见正文"层"那一张表）。
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

// ── 本文件没有一行测试（用户裁定"protocol-case 没必要"）────────────────
//
// 那台编外宿主靶（`roster` 靶里 `principal_core` 那一格）连同 `crates/gate`（已删）的 `host` 那一门
// 已删。原先那七条用例本来就是"编不到、也跑不到"的规格（`protocol` 是 `[lib] test = false`，
// riscv 上编不出 libtest）——它们**短暂地**被跑起来过（从宿主靶开始），如今又回到只有写着的
// 规格。真机上另有探针那几条（`harness/src/subject.rs`）。
