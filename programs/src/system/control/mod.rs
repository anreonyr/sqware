//! system::control — **Service 的生命周期**：建 / 配 / 起 / 停。
//!
//! 它只答一件事：**这一条服务在不在、怎么被创建 / 配置 / 启动 / 停止。**
//! 四套协议（板 / 持树者 / 名册 / 盟册）的语义**不在这一层**：那些是装配单上的**边**，
//! 由 `System` 在装配那一圈按次序落到协议各自的手上。
//!
//! ```text
//!   Program（静态声明）── spawn → connect → start → wire ──▶ Service（域 + 线程 + 通道）
//! ```
//!
//! **`Service` 不另立类型**：它就是"一枚线程 ＋ 它那几条通道"（[`Service`] 是那两样的别名）。
//! 原先那个六格结构体（`name` / `task` / `quay` / `marks` / `needs` / `channel`）是"把一个
//! 函数拆成三个调用点"逼出来的壳——后三格每次都能从 `setup` 现推，`name`/`task` 账里本来就有。
//!
//! 手里只有装配环境四样：账（[`Table`]）、清单、机器自述、与引导域的会话。死亡道与那只组
//! （监督相的东西）住 `System`。
//!
//! **内核那一侧的手住 [`service`]**；**立账与递单住 [`assemble`]**；**监督相住 [`supervise`]**。

use alloc::vec::Vec;

use env::{Mark, Name, TaskId, Wait};
use plan::manifest;
use protocol::communication::establish::{self, Endpoint};
use protocol::system::core::{Fail, Reaped};
use protocol::system::desk::Table;

use crate::root::boot;
use crate::system::machine::Machine;
use crate::system::program::Setup;

pub mod assemble;
pub mod service;
pub mod supervise;

/// 装配失败的编号——定义见 [`plan::assembly::Died`]（本处只是转发）。
pub use plan::assembly::Died;

/// 认身份门牌的短等间隔（毫秒）：门牌由名册起手交出，装配者这一侧只是短等。
pub const RETRY_MS: usize = 1;

/// 等子域就绪/交通道的上限（毫秒）。**必须有界**：子域要是死在头几步，本域不能陪着挂死。
pub const READY_MS: usize = 1000;

/// 装配失败的编号（通用的那几个；按服务分的编号住在装配单旁边）。
pub const E_MANIFEST: Died = 2;
pub const E_PROGRAM: Died = 3;
pub const E_TABLE: Died = 4;

/// **Control 的失败域**：一格 = 死在装配的哪一类。
///
/// **不是全局错误表**：它是装配那一圈的返回类型，号（`env::Reason`）由调用方在边界上折
/// ——按服务分的号归装配单那一格 `died`。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// 名字非法 / 读不懂（`Name::new` 那一关）。
    Manifest,
    /// 清单里没有这一台。
    Missing,
    /// 账里立不起这一行（没登记过 / 表满）。
    Table,
    /// 身子没产出来（建域 / 产线程那一关，或账里没有这一行）。
    Spawn,
    /// 死在装配的某一步——**读数靠 debug 行"程序名 + 哪一步"**。
    Step(&'static str),
}

impl Error {
    /// 一行读数的说法（诊断那一行印的就是它）。
    pub fn said(self) -> &'static str {
        match self {
            Error::Manifest => "bad name",
            Error::Missing => "not in catalog",
            Error::Table => "no table row",
            Error::Spawn => "spawn failed",
            Error::Step(what) => what,
        }
    }
}

/// **一条运行时服务**：一枚线程 ＋ 它那几条通道（会话）。
///
/// 线程与通道都在这里：`spawn` 只挂线程（账是空的），放行前 `connect` 按 `setup` 逐条装
/// （第一条是 `records`），随后板 / 树两条装配路也各往这本账里添一件。
///
/// **一条通道一件持有者**（[`Endpoint`]）——`Endpoint` 只装两枚孔，故"一条关系 N 条通道"那一档
/// 在这里就是**几个 `Endpoint`**，不是一个能装的容器类型。**这本账归装配者拿着**：那几枚孔是
/// 本域铸出去、客人将来要认的那一半，放早了客人就没得认（见 `System::bring_up`）。
/// 名字不进这个别名——账里那一行就是名字，调用方手里也有 `Program`。
pub type Service = (TaskId, Vec<Endpoint>);

/// 清单的读面：装配者按名字挑镜像。
///
/// 两种来源**同一形状**：引导域手里是 boot 借映的那块字节，编排域手里是它从固件领来的
/// 那段只读视图（同一批物理页、各自的 VA）。清单里的镜像是**相对 blob 的切片**，故换一张
/// 表、换一个 VA 都照样解析得出来——这正是"零拷贝把这片区交出去"能成立的原因。
#[derive(Clone, Copy)]
pub struct Catalog<'a> {
    view: &'a [u8],
}

impl<'a> Catalog<'a> {
    /// 拿一块字节当清单。`None` = 清单头非法（条数为零 / 超上限 / 装不下）。
    pub fn new(view: &'a [u8]) -> Option<Catalog<'a>> {
        manifest::Entries::new(view)?;
        Some(Catalog { view })
    }

    /// boot 借映给引导域的那块。
    pub fn of_boot(boot: &boot::Root) -> Option<Catalog<'static>> {
        Catalog::new(boot.view())
    }

    /// 从清单里挑出这个程序。
    pub fn find(&self, want: &str) -> Option<manifest::Entry<'a>> {
        let mut list = self.programs();
        loop {
            let entry = list.next()?;
            let Ok(entry) = entry else { return None };
            if entry.name == want {
                return Some(entry);
            }
        }
    }

    fn programs(&self) -> manifest::Entries<'a> {
        manifest::Entries::new(self.view).expect("清单头已在 new 时验过")
    }
}

/// **Service 的生命周期与装配环境**。
pub struct Control {
    table: Table,
    catalog: Catalog<'static>,
    machine: Machine,
    boot: Endpoint,
}

impl Control {
    /// 就位（四样都由 `System` 在引导之后交进来）。
    pub fn new(catalog: Catalog<'static>, machine: Machine, boot: Endpoint) -> Control {
        Control {
            table: Table::new(),
            catalog,
            machine,
            boot,
        }
    }

    /// **起一个 Service**：按名字去清单里挑镜像 → 建域 → 产线程 → 备通道账。
    ///
    /// 前置：这一行**已经登记过**（[`Control::enlist`]）——没登记过由 `admit_start` 拦下。
    ///
    /// 通道账起手是空的：`setup` 里那几条由 [`connect`] 逐条装上（放行之前）。
    pub fn spawn(&mut self, name: &'static str) -> Result<Service, Error> {
        let name = Name::new(name).map_err(|_| Error::Manifest)?;
        let entry = self.catalog.find(name.as_str()).ok_or(Error::Missing)?;
        let task = service::mint(&mut self.table, name, entry.elf, entry.kind)
            .map_err(|_| Error::Spawn)?;
        Ok((task, Vec::new()))
    }

    /// **放行 + 等就绪 + 认领通道**（有通道的那一条顺带逐条认领）。
    ///
    /// **这一刀之后它就跑了**：门闩、通道都在放行前定下（两相之间的窗口就是"它一步都还没跑"）。
    /// `setup` 里那几条 `Channel` 就是放行后要逐条认领的记号（记号即通道名）。
    pub fn start(
        &mut self,
        name: Name,
        service: &mut Service,
        setup: &'static [Setup],
    ) -> Result<(), Error> {
        let (task, channels) = service;
        let mut marks: Vec<Mark> = Vec::new();
        for s in setup {
            if let Setup::Channel(ch) = s {
                marks
                    .try_reserve(1)
                    .map_err(|_| Error::Step("no room for marks"))?;
                marks.push(Mark::of(ch));
            }
        }
        service::start(
            &mut self.table,
            name,
            *task,
            &[],
            channels.as_mut_slice(),
            &marks,
            Wait::AtMost(READY_MS),
        )
        .map_err(|_| Error::Step("start failed"))
    }

    /// 收掉一条 Service：**下令即回，不等它收完**。
    pub fn stop(&mut self, name: Name) -> Result<(), Fail> {
        service::stop(&mut self.table, name)
    }

    /// 等它收尾（**只读：不动表**）。
    pub fn until(&self, name: Name, wait: Wait) -> Result<Reaped, Fail> {
        service::until(&self.table, name, wait)
    }

    /// 盯它一眼：`true` = 它收尾了（内核的事实优先，表随之落定 `Dead`）。
    pub fn observe(&mut self, name: Name, wait: Wait) -> Result<bool, Fail> {
        service::watch(&mut self.table, name, wait)
    }

    /// 等一条服务退场（本域等它 = 等这次会话结束）。
    pub fn wait_last(&mut self, name: Name) {
        while let Ok(false) = service::watch(&mut self.table, name, Wait::Forever) {}
    }
}

/// **装一条通道**（放行前）：铸本端那一枚（刻 `ch` 的记号）交给这条服务的域、并顺手试认它那一枚
/// （`POLL` = 不等：放行前它一步都还没跑，认不到是常态）——放行后按同一个记号再认一次
/// （[`service::ready`] 逐条 `claim`）。
///
/// 只碰通道、不碰 `Control` 的任何一格，故是自由函数（`Control::connect` 那一层是白加的壳）。
pub fn connect(to: TaskId, ch: &'static str) -> Result<Endpoint, Error> {
    let name = Name::new(ch).map_err(|_| Error::Manifest)?;
    establish::endpoint(to, Mark::of(name.as_str()), Wait::POLL)
        .map_err(|_| Error::Step("connect failed"))
}
