//! Service 编排 — **表 + 状态 + 判定**（核心）与**转发**（适配）。
//!
//! 两层，判据见模块头注：
//!
//! ```text
//!   核心（本文件前半）  表 / 状态 / 判定 —— 不碰内核，只认名字与状态
//!   适配（本文件后半）  转发 —— 调运行时那几件（建域 / 产线程 / 放行 / 等 / 收）
//! ```
//!
//! 核心是纯的：三个判定（`admit_start` / `probe_ready` / `probe_watch`）只读表、不改
//! 状态、不调内核，故可离线喂假表验证；表的每次变更都只经 [`Table`] 的那几个方法
//! （不变量入口）。**"去问内核"一律在适配层**：适配把内核答的事实写回表，核心只解读
//! 表里的事实。

use env::{Name, Permission, PieToken, ProgramKind, TaskId, TeamId};

use crate::session::Quay;

// ── 核心：类型 ──────────────────────────────────────────────

/// Service 的生命阶段。**失败不在这里**——失败由 [`Fail`] 承载（两者是两件事）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// 表里有这一行，但还没起过。
    NeverStarted,
    /// 起了，正在等它宣布就绪。
    Starting,
    /// 已就绪。
    Ready,
    /// 已下令收，还没确认收干净。
    Stopping,
    /// 起过、现在是死的（被收掉或自己走的）。
    Dead,
}

/// 这一次运行的载体：域 + 代表线程。
///
/// **两个句柄绑在同一个变体里**是刻意的：分开成两个字段就允许"有域、没线程"这种
/// 半死状态被写出来，而现在它不可表达。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    None,
    Live { team: TeamId, rep: TaskId },
}

/// **怎么知道它起来了**——每个 Service 自己的一种，**登记时定死**。
///
/// 这一格不能一刀切：有的服务起来时会交回一条通道（那枚句柄的到达就是它的"我好了"），
/// 有的**什么都不交**（比如只走调试面的回显——它没有通道可交）。它是这一行的属性，
/// 不是调用 `start` 时的一个开关，故与名字、身子、状态同住一行。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Announce {
    /// 它会交回一枚句柄 ⇒ 那枚到了才算起来。
    Channel,
    /// 它不宣布 ⇒ **放行即起来**（"起来了"= 它没死）。
    None,
}

/// 表里的一行。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Service {
    /// 服务名（清单名，≤ 31 字节）。**表内的唯一坐标**。
    pub name: Name,
    /// 这一次运行的载体。
    pub slot: Slot,
    /// 生命阶段。
    pub state: State,
    /// 怎么算"起来了"。
    pub announce: Announce,
    /// 它交回来的通道句柄（未交 = `None`）。`Announce::Channel` 的就绪证据就是它。
    pub root: Option<PieToken>,
}

/// 一行的初值（表是定长数组，故要一个可复制的空行）。
const EMPTY: Service = Service {
    name: Name::EMPTY,
    slot: Slot::None,
    state: State::NeverStarted,
    announce: Announce::None,
    root: None,
};

/// Service 表：**定长、线性查**。
///
/// 上限 16 与程序清单同值（装不进机器的程序，也起不出服务）；三五个服务不需要哈希，
/// 也就省掉一层分配——本表可以在没有任何堆的宿主上存在。
pub struct Table {
    rows: [Service; Table::CAP],
}

impl Table {
    /// 行数上限（= `env::wire::manifest::MAX_PROGRAMS`）。
    pub const CAP: usize = 16;

    /// 空表：每一行都"占着位但没名字"。
    pub const fn new() -> Table {
        Table {
            rows: [EMPTY; Table::CAP],
        }
    }

    /// **登记一行**：只知道名字与它"怎么算起来"——此刻还没有身子。
    ///
    /// 这是"起之前"唯一的入口；身子由 [`Table::attach`] 在真的起了之后挂上。
    pub fn register(&mut self, name: Name, announce: Announce) -> Result<(), Fail> {
        if self.find(name).is_some() {
            return Err(Fail::Unknown);
        }
        let Some(row) = self.rows.iter_mut().find(|s| s.name.is_empty()) else {
            return Err(Fail::NoRoom);
        };
        row.name = name;
        row.announce = announce;
        Ok(())
    }

    /// 按名字找那一行（没名字的行不算）。
    pub fn find(&self, name: Name) -> Option<&Service> {
        self.rows.iter().find(|s| s.name == name)
    }

    /// 全部有名字的行（含没起过的）——枚举的读面。
    pub fn rows(&self) -> impl Iterator<Item = &Service> {
        self.rows.iter().filter(|s| !s.name.is_empty())
    }

    /// 改状态。找不到 = 名字不对 ⇒ 不动任何东西。
    pub fn set_state(&mut self, name: Name, state: State) {
        if let Some(s) = self.row_mut(name) {
            s.state = state;
        }
    }

    /// 记下它交回的通道。
    pub fn hand_over(&mut self, name: Name, root: PieToken) {
        if let Some(s) = self.row_mut(name) {
            s.root = Some(root);
        }
    }

    /// 挂上身子：**一次给全**（域 + 线程）。没登记过 ⇒ `Unknown`。
    pub fn attach(&mut self, name: Name, team: TeamId, rep: TaskId) -> Result<(), Fail> {
        let Some(s) = self.row_mut(name) else {
            return Err(Fail::Unknown);
        };
        s.slot = Slot::Live { team, rep };
        s.root = None;
        s.state = State::NeverStarted;
        Ok(())
    }

    /// 摘掉身子与通道（行留着：状态要能说出"起过、现在死了"）。
    pub fn detach(&mut self, name: Name) {
        if let Some(s) = self.row_mut(name) {
            s.slot = Slot::None;
            s.root = None;
        }
    }

    fn row_mut(&mut self, name: Name) -> Option<&mut Service> {
        self.rows.iter_mut().find(|s| s.name == name)
    }
}

// ── 核心：判定（纯函数，只读表）─────────────────────────────

/// 起一个 Service 的**准许**：只有它没在跑、也不在停，才准起。
///
/// 两种拒绝的理由不同，故不压成一个：名字不在表里 = 调用方写错了；
/// 它已经在跑/在停 = 时机不对。
pub fn admit_start(table: &Table, name: Name) -> Result<(), Fail> {
    let Some(s) = table.find(name) else {
        return Err(Fail::Unknown);
    };
    match s.state {
        State::NeverStarted | State::Dead => Ok(()),
        State::Starting | State::Ready | State::Stopping => Err(Fail::NotReady),
    }
}

/// "起来了没有"的三态判定。**只读表**——"它交回句柄了没有"是内核的事实，由适配写进
/// 表之后这里才读得到（见 [`ready`]）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ready {
    /// 已就绪（它宣布过了，或它这一种根本不需要宣布）。
    Up,
    /// 代表线程已经不在了 ⇒ 它没起来。
    Gone,
    /// 既没宣布、也还活着 ⇒ 继续等。
    Pending,
}

/// 就绪判定（纯）：按**这一行自己声明的**说法解读。
pub fn probe_ready(table: &Table, name: Name) -> Ready {
    let Some(s) = table.find(name) else {
        return Ready::Gone;
    };
    match s.state {
        State::Ready => Ready::Up,
        State::Starting => match (s.announce, s.slot) {
            // 不宣布的那一种：放行就算起来了（它死了会走 `Dead`，那时是 `Gone`）。
            (Announce::None, Slot::Live { .. }) => Ready::Up,
            (Announce::Channel, Slot::Live { .. }) => Ready::Pending,
            (_, Slot::None) => Ready::Gone,
        },
        State::Stopping => Ready::Gone,
        State::NeverStarted | State::Dead => Ready::Gone,
    }
}

/// 还活着没有。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Watch {
    Alive,
    Gone,
}

/// 存活判定（纯）：表里还有身子就算活着——**身子的摘除由适配在确认回收后做**。
pub fn probe_watch(table: &Table, name: Name) -> Watch {
    match table.find(name) {
        Some(Service {
            slot: Slot::Live { .. },
            ..
        }) => Watch::Alive,
        _ => Watch::Gone,
    }
}

// ── 失败域 ──────────────────────────────────────────────────

/// 原语失败的**四种**（对应"调用方接下来该干什么"，不是"内核哪一步坏了"）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    /// 表里没这个名字，或它已经登记过。
    Unknown,
    /// 镜像装不上。
    BadImage,
    /// 表满，或线程/帧产不出来。
    NoRoom,
    /// 没就绪：等到期还没起来、半路死了、或此刻不该起（已在跑）。
    NotReady,
}

// ── 适配：原语（转发到运行时那几件）──────────────────────────

/// 起跑前要交出去的一枚门闩：给哪一枚、多大权。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Grant {
    /// 要交出去的那一枚（**在我表里**的句柄）。
    pub token: PieToken,
    /// 交出去的权限子集。
    pub perm: Permission,
}

/// 起一个 Service。
///
/// `image` = 镜像字节（v1 口径）；`kind` = 特权级（**由清单决定**，调用方转交）；
/// `grants` = **放行前**要交到它手里的门闩（空 = 什么都不预先给）；
/// `quay` = 与它的会话（`Announce::Channel` 那一种才有：它就绪的凭据长在这里）；
/// `ms` = 就绪等待（`0` 只探、`usize::MAX` 无期限、其余毫秒）。
///
/// **失败时不留下半行**：产线程**之前**失败 ⇒ 表不动；产线程**之后**失败 ⇒ 实例与
/// 状态都如实留在表里（它确实在跑），调用方用 [`stop`] 收尾。
pub fn spawn(
    table: &mut Table,
    name: Name,
    image: &[u8],
    kind: ProgramKind,
) -> Result<TaskId, Fail> {
    admit_start(table, name)?;

    let team = crate::system::call::mint(image, kind, name)?;
    let Ok(rep) = crate::system::call::bear(team) else {
        return Err(Fail::NoRoom);
    };
    table.attach(name, team, rep)?;
    table.set_state(name, State::Starting);
    Ok(rep)
}

/// 第二相：**放行**，并在放行前塞门闩、定会话。
///
/// `grants` = 放行前要交到它手里的门闩（空 = 什么都不预先给）；`quay` = 与它的会话
/// （`Announce::Channel` 那一种要用：它就绪的凭据长在这里）；`ms` = 就绪等待
/// （`0` 只探、`usize::MAX` 无期限、其余毫秒）。
///
/// **两相之间的窗口就是"它一步都还没跑"**——塞门闩、定会话都发生在这段窗口里，这与
/// 载体的"恒产未放行"是同一条。
///
/// **失败时不留下半行**：`spawn` 失败 ⇒ 表不动；本相失败 ⇒ 实例与状态如实留在表里
/// （它确实在跑），调用方用 [`stop`] 收尾。
///
/// # Errors
/// 见 [`Fail`]。
pub fn start(
    table: &mut Table,
    name: Name,
    rep: TaskId,
    grants: &[Grant],
    mut quay: Option<&mut Quay>,
    ms: usize,
) -> Result<(), Fail> {
    let launched = (|| -> Result<(), Fail> {
        if let Some(q) = quay.as_deref_mut() {
            // 对端**只由调用方定**：孩子把孔交给"生我者"（建域那一枚线程），
            // 而 `rep` 是刚产出的代表线程——两者不是同一个，这里不能替它改。
            // 这里只确认"已经定过"，没定就是调用方漏了。
            q.peer().ok_or(Fail::Unknown)?;
        }
        for g in grants {
            crate::system::call::accord(g.token, rep, g.perm)?;
        }
        crate::system::call::hatch(rep)
    })();
    if let Err(e) = launched {
        crate::system::call::ruin(rep);
        table.detach(name);
        table.set_state(name, State::Dead);
        return Err(e);
    }
    ready(table, name, quay.as_deref_mut(), ms)?;
    Ok(())
}

/// 等它就绪。`true` = **调用开始时就已就绪**（挂起过的一律 `false`）。
///
/// 唯一会改表的地方就是这个函数：确认它的宣布之后置 [`State::Ready`]，确认它死了之后
/// 落 [`State::Dead`]——**内核的事实在这里变成表里的事实**。而"它宣布了"那件事本身
/// **归 [`session`](crate::session)**：会话在对方交回孔并归位时成立，本函数只去问那句
/// "成立了没有"。
pub fn ready(
    table: &mut Table,
    name: Name,
    quay: Option<&mut Quay>,
    ms: usize,
) -> Result<bool, Fail> {
    // 先看表：上一次问过的事实（按这一行自己声明的说法解读）。
    if let Ready::Up = probe_ready(table, name) {
        return Ok(true);
    }
    let Some(Service {
        slot: Slot::Live { rep, .. },
        announce,
        ..
    }) = table.find(name)
    else {
        return Err(Fail::NotReady);
    };
    let (rep, announce) = (*rep, *announce);

    // 它不宣布的那一种：放行之后只要还活着就算起来了，没有可等的东西。
    if announce == Announce::None {
        if crate::system::call::running(rep) {
            table.set_state(name, State::Ready);
            return Ok(false);
        }
        table.detach(name);
        table.set_state(name, State::Dead);
        return Err(Fail::NotReady);
    }

    // 它会交回一枚孔 ⇒ 那件事归会话：`claim` 把它认下来（凑齐了才算）。
    if let Some(q) = quay {
        // 它会交回孔、也会自己装一条 ⇒ 那件事归会话：`claim` 等**它装上的每条都配齐**。
        // 认领的对端由会话自己记着（那是"建它那个域的那枚线程"，不是 `rep`）。
        let Some(peer) = q.peer() else {
            return Err(Fail::NotReady);
        };
        if q.claim(peer, ms).is_ok() {
            table.set_state(name, State::Ready);
            return Ok(false);
        }
    }
    if !crate::system::call::running(rep) {
        table.detach(name);
        table.set_state(name, State::Dead);
        return Err(Fail::NotReady);
    }
    // 还活着、只是没宣布：它确实在跑，**实例如实留在表里**（调用方用 `stop` 收尾）。
    if ms == 0 {
        return Ok(false);
    }
    Err(Fail::NotReady)
}

/// 收掉一个 Service。**下令即回，不等它收完**。
pub fn stop(table: &mut Table, name: Name) -> Result<(), Fail> {
    let Some(Service {
        slot: Slot::Live { rep, .. },
        ..
    }) = table.find(name)
    else {
        return Err(Fail::Unknown);
    };
    let rep = *rep;
    crate::system::call::ruin(rep);
    table.set_state(name, State::Stopping);
    Ok(())
}

/// 盯着它：`true` = **调用开始时**它已经没了。
///
/// `ms` 三态与 [`ready`] 同款：`0` 只探、`usize::MAX` 挂到它收尾、其余毫秒。
pub fn watch(table: &mut Table, name: Name, ms: usize) -> Result<bool, Fail> {
    let Some(Service {
        slot: Slot::Live { rep, .. },
        ..
    }) = table.find(name)
    else {
        return Err(Fail::Unknown);
    };
    let rep = *rep;

    // 内核的事实优先：它说收了就是收了，表随之落定。
    if crate::system::call::until(rep, ms)? {
        table.detach(name);
        table.set_state(name, State::Dead);
        return Ok(true);
    }
    Ok(false)
}

/// 按名字找那一行的只读视图（枚举的入口）。
pub fn find(table: &Table, name: Name) -> Option<&Service> {
    table.find(name)
}
