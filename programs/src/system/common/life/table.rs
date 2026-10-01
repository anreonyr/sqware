//! common::life::table — **生命轴那一本账**：一张定长表（`Service`）与一行的形状（名字 / 身子 /
//! 生命阶段 / 怎么算起来）。
//! 它只服务**生命轴**（[`Control`](super::Control) 与它那四相）与几处读口（板与服务那一侧的
//! "它还在不在"、`src/harness/` 那几台测具）。**那一册待客账不在这里**：它是**两枚域共用**的一本
//! （板线程在编排域、持树者在 operator 域），故住它们共同的那一格 `crate::system::common::face::desk`。

use alloc::string::String;
use env::{TaskId, TeamId};

use crate::unit::Ending;

use super::verdict::Fail;

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
    Dead,
}

/// **最近一次实例的坐标**：域 + 那一枚线程。生死看 [`State`]——`State::Dead` 与坐标并存
/// 是合法的（"起过、现在死了"），坐标留给重启与放下用：**死亡记账不清它**（清了就没得
/// 放下、也没得重启）。
/// **两格绑在同一个变体里**是刻意的：分开成两个字段就允许"有域、没线程"这种半死状态
/// 被写出来，而现在它不可表达。
/// `team = None` = **那一枚线程住本域**（iii：编排域的四枚线程里，除编排者自己以外那三枚）。
/// 这个 `None` 不是"省一格"：`team` 的**唯一读者**是 `mark_dead` 那一格（要"放下那个域"），
/// 而本域那一枚**没有别人的域可放下**——放下它就是扑杀本域自己（板线程那一格量过：
/// 而本域那一枚**没有别人的域可放下**——放下它就是扑杀本域自己（量过：板线程那一格 `system: done` 在 1005 份 soak 日志里一次都没有）。

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    None,
    Live { team: Option<TeamId>, task: TaskId },
}

/// **怎么知道它起来了**——每个 Service 自己的一种，**登记时定死**。
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
/// `restart` 那一格对空行**没有意义**：[`Table::rows`] 把没名字的行滤掉，判定看不到它。
const EMPTY: Service = Service {
    name: String::new(),
    slot: Slot::None,
    state: State::NeverStarted,
    announce: Announce::None,
    restart: Ending::Resident,
};

/// Service 表：**定长、线性查**。
/// 上限与程序清单同值（装不进机器的程序，也起不出服务）；三五个服务不需要哈希，
/// 也就省掉一层分配——本表可以在没有任何堆的宿主上存在。
pub struct Table {
    rows: [Service; Table::CAP],
}

impl Table {
    /// 行数上限（= `env::ledger::manifest::MAX_PROGRAMS`）。
    pub const CAP: usize = 28;

    /// 空表：每一行都"占着位但没名字"。
    pub const fn new() -> Table {
        Table {
            rows: [EMPTY; Table::CAP],
        }
    }

    /// **登记一行**：只知道名字、它"怎么算起来"、以及**谁结束它**——此刻还没有身子。
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
