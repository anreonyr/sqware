//! principal::core — **名册 ＋ 谱系那本账**：两张表与一把钥匙。
//!
//! **照实记（它原先住 `protocol::service::principal::core`）**：那一份的读者只有本域的持有者
//! （`prog-principal` 那一枚线程）——按"协议 = 共享语言"的判据，它属于实现侧。协议那一侧
//! 只留**上线的两样**：号（`PrincipalId`）与失败域（`Fail`）。
//!
//! 本文件只讲两张表的形状与九条原语；**身份的含义、门牌、帧那一层**写在协议那一边
//! （`protocol::service::principal`）。

use alloc::vec::Vec;

use env::TaskId;

use protocol::service::principal::{Fail, PrincipalId};

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
        let id = PrincipalId::new(self.tree.len());
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
