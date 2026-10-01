//! control::desk — **生命轴那一本账**：一张定长表（`Service`）与一行的形状（名字 / 身子 /
//! 生命阶段 / 怎么算起来）。
//!
//! 它只服务**生命轴**（[`Control`](super::Control) 与它那四相）与几处读口（板与服务那一侧的
//! "它还在不在"、`harness` 那几台测具）。**那一册待客账不在这里**：它是**两枚域共用**的一本
//! （板线程在编排域、持树者在 operator 域），故住它们共同的那一格 `crate::system::desk`。
//!
//! **照实记（这一刀：两份账分家）**：两本原先同住 `programs/src/system/desk.rs`，而那个文件的
//! 头注只描述了这一本（"一张定长表与一行的形状"）——**文件头说不全的，就是放错了地方**。
//! 判定那四条（起不起 / 起来了没有 / 收尾完了没有）跟着这本账走：住 [`core`](super::core)。

use alloc::string::String;
use env::{TaskId, TeamId};

use crate::unit::Ending;

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

/// **最近一次实例的坐标**：域 + 那一枚线程。生死看 [`State`]——`State::Dead` 与坐标并存
/// 是合法的（"起过、现在死了"），坐标留给重启与放下用：**死亡记账不清它**（清了就没得
/// 放下、也没得重启）。
///
/// **两格绑在同一个变体里**是刻意的：分开成两个字段就允许"有域、没线程"这种半死状态
/// 被写出来，而现在它不可表达。
///
/// `team = None` = **那一枚线程住本域**（iii：编排域的四枚线程里，除编排者自己以外那三枚）。
/// 这个 `None` 不是"省一格"：`team` 的**唯一读者**是 `mark_dead` 那一格（要"放下那个域"），
/// 而本域那一枚**没有别人的域可放下**——放下它就是扑杀本域自己（板线程那一格量过：
/// `system: done` 在 1005 份 soak 日志里一次都没有）。把"没有别人的域"写成 `None`，
/// 那一刀就写不出来。
///
/// **照实记（订正：那三枚今天不在本域了）**：同域 `spawn_here`（`TeamId(0)`）已随 `0560dd8`
/// 退场——那三枚回普通程序、各成各的域，故 `team = None` 这一格**今天仓内没有生产者**
/// （`service::mint` 只写 `Some(team)`）。该状态仍经 [`Table::attach`] 的 `team = None` 可表达，
/// 故它是一格**护栏、不是活路径**（同一句也记在 `supervise.rs::stop_running` 那一支旁，两处
/// 不许各说各的）；代价照实说：它今天编得过、走不到，没有一个用例钉着。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    None,
    Live { team: Option<TeamId>, task: TaskId },
}

/// **怎么知道它起来了**——每个 Service 自己的一种，**登记时定死**。
///
/// 这一格不能一刀切：有的服务起来时会交回一条通道（那枚句柄的到达就是它的"我好了"），
/// 有的**什么都不交**（比如只走调试面的回显——它没有通道可交）。它是**装配声明**上的一格
/// （`programs` 的 `Demand::setup` 里有没有 `Channel` 推出来），不是调用 `start` 时的一个
/// 开关。
///
/// **照实记（它为什么住这里）**：它原住本模块，被搬去 `crates/plan` 那张装配表旁边
/// （理由是"装配表要摆出这一格"）；装配表退场之后它回来了——本模块是它的唯一定义处，
/// 两侧（装配侧与表那一侧）都从这里取。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Announce {
    /// 它会交回一枚句柄 ⇒ 那枚到了才算起来。
    Channel,
    /// 它不宣布 ⇒ **放行即起来**（"起来了"= 它没死）。
    None,
}

/// 表里的一行。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Service {
    /// 服务名（清单名，≤ 31 字节）。**表内的唯一坐标**。
    pub name: String,
    /// 这一次运行的载体。
    pub slot: Slot,
    /// 生命阶段。
    pub state: State,
    /// 怎么算"起来了"。
    pub announce: Announce,
    /// **谁结束它**——登记时定死（与 [`Announce`] 同款：两格都是声明里推出的事实，落在行上之后
    /// 账就自足——收场那三条判定（`core::{due, done, walking}`）只读账，不必回头查声明）。
    pub restart: Ending,
}

/// 一行的初值（表是定长数组，故要一个可复制的空行）。
///
/// `restart` 那一格对空行**没有意义**：[`Table::rows`] 把没名字的行滤掉，判定看不到它。
const EMPTY: Service = Service {
    name: String::new(),
    slot: Slot::None,
    state: State::NeverStarted,
    announce: Announce::None,
    restart: Ending::Resident,
};

/// Service 表：**定长、线性查**。
///
/// 上限与程序清单同值（装不进机器的程序，也起不出服务）；三五个服务不需要哈希，
/// 也就省掉一层分配——本表可以在没有任何堆的宿主上存在。
pub struct Table {
    rows: [Service; Table::CAP],
}

impl Table {
    /// 行数上限（= `env::manifest::MAX_PROGRAMS`）。
    pub const CAP: usize = 28;

    /// 空表：每一行都"占着位但没名字"。
    pub const fn new() -> Table {
        Table {
            rows: [EMPTY; Table::CAP],
        }
    }

    /// **登记一行**：只知道名字、它"怎么算起来"、以及**谁结束它**——此刻还没有身子。
    ///
    /// 这是"起之前"唯一的入口；身子由 [`Table::attach`] 在真的起了之后挂上。
    pub fn register(
        &mut self,
        name: String,
        announce: Announce,
        restart: Ending,
    ) -> Result<(), Fail> {
        if self.find(name.as_str()).is_some() {
            return Err(Fail::Unknown);
        }
        let Some(row) = self.rows.iter_mut().find(|s| s.name.is_empty()) else {
            return Err(Fail::Full);
        };
        row.name = name;
        row.announce = announce;
        row.restart = restart;
        Ok(())
    }

    /// 按名字找那一行（没名字的行不算）。**只查不比存** ⇒ 借 `&str`。
    pub fn find(&self, name: &str) -> Option<&Service> {
        self.rows.iter().find(|s| s.name == name)
    }

    /// 全部有名字的行（含没起过的）——枚举的读面。
    pub fn rows(&self) -> impl Iterator<Item = &Service> {
        self.rows.iter().filter(|s| !s.name.is_empty())
    }

    /// **还活着的行**——`Dead` 之外的一切。
    ///
    /// **"活着"只有这一句**：`Stopping`（已下令收、还没确认收干净）**算活着**——收场那三条判定
    /// 的前提正是它；少了它，被线上 `Stop` 推入 `Stopping` 而道又不响的行会永远不落 `Dead`。
    pub fn living(&self) -> impl Iterator<Item = &Service> {
        self.rows().filter(|s| !matches!(s.state, State::Dead))
    }

    /// 改状态。找不到 = 名字不对 ⇒ 不动任何东西。
    pub fn set_state(&mut self, name: &str, state: State) {
        if let Some(s) = self.row_mut(name) {
            s.state = state;
        }
    }

    /// 挂上身子：**一次给全**（域 + 线程）。没登记过 ⇒ `Unknown`。
    ///
    /// `team = None` = 那一枚线程**住本域**（见 [`Slot`]）。
    pub fn attach(&mut self, name: &str, team: Option<TeamId>, task: TaskId) -> Result<(), Fail> {
        let Some(s) = self.row_mut(name) else {
            return Err(Fail::Unknown);
        };
        s.slot = Slot::Live { team, task };
        s.state = State::NeverStarted;
        Ok(())
    }

    /// 摘掉身子（行留着：状态要能说出"起过、现在死了"）。
    pub fn detach(&mut self, name: &str) {
        if let Some(s) = self.row_mut(name) {
            s.slot = Slot::None;
        }
    }

    fn row_mut(&mut self, name: &str) -> Option<&mut Service> {
        self.rows.iter_mut().find(|s| s.name == name)
    }
}
