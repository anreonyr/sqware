//! system::desk — **账**：一张定长表与一行的形状（名字、身子、生命阶段、就绪凭据）
//!
//! 正文见 [`super`]；三档（判定 / 账 / 适配）分家的理由见 `system` 模块头注。

use env::{Name, PieToken, TaskId, TeamId};

use super::core::Fail;

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

/// **最近一次实例的坐标**：域 + 代表线程。生死看 [`State`]——`State::Dead` 与坐标并存
/// 是合法的（"起过、现在死了"），坐标留给重启与放下用：**死亡记账不清它**（清了就没得
/// 放下、也没得重启）。
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
/// 上限与程序清单同值（装不进机器的程序，也起不出服务）；三五个服务不需要哈希，
/// 也就省掉一层分配——本表可以在没有任何堆的宿主上存在。
pub struct Table {
    rows: [Service; Table::CAP],
}

impl Table {
    /// 行数上限（= `env::wire::manifest::MAX_PROGRAMS`）。
    pub const CAP: usize = 20;

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
