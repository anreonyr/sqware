//! system::control — **把 Program 变成 Service 的那台机器**。
//!
//! 它只答一件事：**Service 在不在、怎么被创建 / 配置 / 启动 / 停止。**
//! 四套协议（板 / 持树者 / 名册 / 盟册）的语义**不在这一层**：那些是 Scenario 里的**边**，
//! 由 `System` 解释（谁上树、谁上板、谁是名册那一双眼睛、装配期给不给身份）。
//!
//! ```text
//!   Program（静态声明）── assemble() ──▶ Control ──▶ Service（运行时实例）
//! ```
//!
//! 这一层与旧 `service.rs` 的对应：`mint` / `spawn_here` → [`Control::spawn`]；
//! 资源与通道的登记 → [`Control::connect`] / [`Setup`](crate::system::program::Setup)；
//! `start` / `hatch` / `ready` → [`Control::start`]；`stop` / `doom` → [`Control::stop`]；
//! `reaped` / `until` / `watch` → [`Control::observe`] / [`Control::until`]。
//!
//! **内核那一侧住 [`service`]**（建域 / 产线程 / 放行 / 收 / 盯那七手与几枚原语）；
//! **监督相住 [`supervise`]**；**立账与递单住 [`assemble`]**。

use alloc::vec::Vec;

use env::{Mark, Name, PieToken, TaskId, Wait};
use plan::manifest;
use protocol::session::{Pier, Quay};
use protocol::system::core::{Fail, Reaped};
use protocol::system::desk::Table;

use crate::root::boot;
use crate::system::machine::Machine;
use crate::system::program::Source;
use runtime::core::pile::Pile;

pub mod assemble;
pub mod service;
pub mod supervise;

/// 装配失败的编号——定义见 [`plan::assembly::Died`]（本处只是转发）。
pub use plan::assembly::Died;

/// 认身份门牌 / 与它说话的短等间隔（毫秒）：门牌由 Server 起手交出，这里只是短等。
pub const RETRY_MS: usize = 1;

/// 等子域就绪/交通道的上限（毫秒）。**必须有界**：子域要是死在头几步，本域不能陪着挂死。
pub const READY_MS: usize = 1000;

/// 装配失败的编号（通用的那几个；按服务分的编号住在各自的装配单旁边）。
pub const E_MANIFEST: Died = 2;
pub const E_PROGRAM: Died = 3;
pub const E_TABLE: Died = 4;

/// **Control 的失败域**：一格 = 死在装配的哪一类。
///
/// **不是全局错误表**：它是 `Program::assemble` 与 `Setup::apply` 的返回类型，
/// 号（`env::Reason`）由调用方在边界上折——**按服务分的号归装配单**（scenario 的 `Edges`）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// 名字非法 / 读不懂（`Name::new` 那一关）。
    Manifest,
    /// 清单里没有这一台（`Source::Catalog` 那一支）。
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

/// **一条 Service 的运行时句柄**：身子（名字 + 线程）＋ 它的码头（会话）＋ 装配期待办。
///
/// **码头每条服务都开**（不是有通道才开）：板那两条路（板 / 树）也建在同一座码头上
/// （[`board::attach`](crate::system::board::bridge::attach) /
/// [`operator::attach`](crate::system::operator::bridge::attach)），即使这一台
/// `setup` 里一条 `Channel` 都没有。
pub struct Service {
    /// 服务名（清单名）。
    pub name: Name,
    /// 身子那一枚线程。
    pub task: TaskId,
    /// 与它的会话（泊位都装在它身上）。
    quay: Quay,
    /// 放行后要逐条认领的记号（= 通道名）——`Announce::Channel` 的就绪证据。
    marks: Vec<Mark>,
    /// 起来之后要递的门闩单（按 `setup` 的次序；空 = 不要资源）。
    needs: Vec<plan::supply::Need>,
    /// **第一条通道**（递单走它）——旧 `wire` 读的就是 `channels.first()`。
    channel: Option<Name>,
}

impl Service {
    /// 它的码头（板 / 树那两条路要用；`Program` 侧不碰）。
    pub fn quay_mut(&mut self) -> &mut Quay {
        &mut self.quay
    }

    /// 记下"起来之后要领这一枚"（`Setup::Need` 落到这里）。
    pub fn need(&mut self, need: plan::supply::Need) -> Result<(), Error> {
        self.needs.try_reserve(1).map_err(|_| Error::Step("no room for wants"))?;
        self.needs.push(need);
        Ok(())
    }

    /// 它要的那张门闩单（按次序）。
    pub fn needs(&self) -> &[plan::supply::Need] {
        &self.needs
    }

    /// 递单走哪条通道（`None` = 这一台没有通道）。
    pub fn channel(&self) -> Option<Name> {
        self.channel
    }
}

/// **一条死亡道**：哪一位 + 那一条路（本域铸的孔，记号 `gone-<名字>`）。
///
/// **照实记（为什么按名字，不按下标）**：原先道与装配单**按下标**对齐（`lanes[i]` ↔ `plan[i]`，
/// `supervise` 又按同一个下标把"哪条道响"翻回名字）——`assemble/` 那一间早就记过这条耦合的代价
/// （"两张表必须各自自洽……第一版想'表里插一条空名字的行'，默认台当场以 `system: manifest bad`
/// 收场"）。iii 让装配单变成**两段相接**（内件 ＋ 镜像里那几台），跨两段维持"位次自洽"正是
/// 那条隐患复发的地方 ⇒ 改成**按名字**（板那一侧本来就是按记号 `gone-<名字>` 认领的）。
pub struct Lane {
    /// 这一位是谁（装配单上的名字）。
    pub name: &'static str,
    /// 那一条道。`None` 有**两条来路**：**这一位不上板**（边里 `board = false`——道是
    /// 板写的，没有写端就不铸）或**本域铸不出孔**（交给退场级联）。
    pub road: Option<PieToken>,
}

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
///
/// 手里是"这台机器上起服务要用的几样东西"：账（`Table`）、清单、机器自述、与引导域的会话、
/// 死亡道、等任一道响的组。**没有协议状态**——树 / 身份面 / 协调帧那几格住 `System`。
pub struct Control {
    table: Table,
    catalog: Catalog<'static>,
    machine: Machine,
    boot: Pier,
    lanes: Vec<Lane>,
    pile: Pile,
}

impl Control {
    /// 就位（几样东西都由 `System` 在引导之后交进来）。
    pub fn new(
        catalog: Catalog<'static>,
        machine: Machine,
        boot: Pier,
        lanes: Vec<Lane>,
        pile: Pile,
    ) -> Control {
        Control {
            table: Table::new(),
            catalog,
            machine,
            boot,
            lanes,
            pile,
        }
    }

    /// **起一个 Service**：身子的两条来路各一次（建域 / 本域产线程），并把码头开好。
    ///
    /// 前置：这一行**已经登记过**（[`Control::enlist`]）——没登记过由 `admit_start` 拦下。
    pub fn spawn(&mut self, name: &'static str, source: Source) -> Result<Service, Error> {
        let name = Name::new(name).map_err(|_| Error::Manifest)?;
        let task = match source {
            Source::Catalog(want) => {
                let entry = self.catalog.find(want).ok_or(Error::Missing)?;
                service::mint(&mut self.table, name, entry.elf, entry.kind)
                    .map_err(|_| Error::Spawn)?
            }
            Source::Here(role) => {
                service::spawn_here(&mut self.table, name, role).map_err(|_| Error::Spawn)?
            }
        };
        Ok(Service {
            name,
            task,
            quay: Quay::open(task, protocol::session::call::hands()),
            marks: Vec::new(),
            needs: Vec::new(),
            channel: None,
        })
    }

    /// **装一条通道**（放行前）：在它的码头上装一条泊位，并把记号记下（放行后逐条认领）。
    ///
    /// **第一条**记进 `channel`：递单走它（旧 `wire` 读的就是 `channels.first()`）。
    pub fn connect(&mut self, service: &mut Service, ch: &'static str) -> Result<(), Error> {
        let name = Name::new(ch).map_err(|_| Error::Manifest)?;
        service
            .quay
            .seat(name)
            .map_err(|_| Error::Step("seat failed"))?;
        service
            .marks
            .try_reserve(1)
            .map_err(|_| Error::Step("no room for marks"))?;
        service.marks.push(Mark::of(name.as_str()));
        if service.channel.is_none() {
            service.channel = Some(name);
        }
        Ok(())
    }

    /// **放行 + 等就绪 + 认领通道**（有通道的那一条顺带逐条认领）。
    ///
    /// **这一刀之后它就跑了**：门闩、会话都在放行前定下（两相之间的窗口就是"它一步都还没跑"）。
    pub fn start(&mut self, service: &mut Service) -> Result<(), Error> {
        let ups = if service.marks.is_empty() {
            None
        } else {
            Some(&mut service.quay)
        };
        service::start(
            &mut self.table,
            service.name,
            service.task,
            &[],
            ups,
            &service.marks,
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

    /// 这一位的死亡道（按名字取，不是按下标：见 [`Lane`]）。
    pub fn lane_of(&self, name: &str) -> Option<PieToken> {
        self.lanes.iter().find(|l| l.name == name).and_then(|l| l.road)
    }
}
