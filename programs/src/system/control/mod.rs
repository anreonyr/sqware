//! Control 管理外部 team 的创建、配置、启动、停止与监督。

use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;

use ::core::time::Duration;

use crate::system::common::life::table::{Slot, State, Table};
use crate::system::common::life::verdict::Fail;
use env::{Mark, TaskId, Wait};
use protocol::communication::session::establish::{self, Endpoint};

use crate::boot::{Accounts, Catalog};
use crate::system::identity::bridge::Roster;
use crate::system::common::machine::Machine;
use crate::system::run::source::Source;
use crate::unit::{PROGRAMS, Setup, UnitFile};

use self::enroll as assemble;
use crate::system::common::life::{service, verdict as core};

pub mod enroll;
pub mod supervise;

pub use crate::unit::Died;

/// 认身份门牌的短等间隔（毫秒）：门牌由名册起手交出，装配者这一侧只是短等
pub const RETRY_MS: usize = 1;

pub const READY_MS: usize = 1000;

/// **等"它起完了"那一条通道的上限**（毫秒）——比 READY_MS 宽得多
pub const BOOT_MS: usize = 5000;

/// 装配失败的编号（通用的那几个；按服务分的编号住在装配表旁边）
pub const E_MANIFEST: Died = 2;
pub const E_PROGRAM: Died = 3;
pub const E_TABLE: Died = 4;

/// **Control 的失败域**：一格 = 死在装配的哪一类
/// **不是全局错误表**：它是装配那一圈的返回类型，号（env::Reason）由调用方在边界上折
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    Manifest,
    /// 清单里没有这一台
    Missing,
    /// 账里立不起这一行（没登记过 / 表满）
    Table,
    /// 身子没产出来（建域 / 产task那一关，或账里没有这一行）
    Spawn,
    /// 死在装配的某一步——**读数靠 debug 行"程序名 + 哪一步"**
    Step(&'static str),
}

impl Error {
    /// 一行读数的说法（诊断那一行印的就是它）
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

/// **一条运行时服务**：一枚task ＋ 它那几条通道（会话）
/// （第一条是 `records`），随后板 / 树两条装配路也各往这本账里添一件
/// **一条通道一件持有者**（Endpoint）——`Endpoint` 只装两枚孔，故"一条关系 N 条通道"那一档
/// 名字不进这个别名——账里那一行就是名字，调用方手里也有 `UnitFile`
pub type Service = (TaskId, Vec<Endpoint>);

/// **Service 的生命周期与装配环境**
pub struct Control {
    pub(crate) status: alloc::sync::Arc<super::Status>,
    pub(crate) table: Table,
    publication_entry: Option<env::PieToken>,
    pub(crate) roster: Roster,
    pub(crate) activation: Option<crate::service::hub::bridge::Activation>,
    pub(crate) static_tasks: Vec<TaskId>,
    pending: Vec<Pending>,
    catalog: Catalog<'static>,
    pub(crate) machine: Machine,
    accounts: Accounts,
    /// **上一手入册那一单的写端**（Setup::Machine 那一格）
    out: protocol::communication::hand::Sender<protocol::service::hub::Enroll>,
}

/// 一枚**已造未放行**的身子（Control::pending 那一格）
struct Pending {
    /// 装配表上的名字（`&'static str`——`spawn` / `start` 要它）
    name: &'static str,
    /// 那一枚task ＋ 它已经认领的通道
    service: Service,
}

impl Control {
    /// 就位（四样都由 `System` 在引导之后交进来）
    pub fn new(catalog: Catalog<'static>, machine: Machine, accounts: Accounts, status: alloc::sync::Arc<super::Status>, publication_entry: Option<env::PieToken>) -> Control {
        Control {
            status,
            table: Table::new(),
            publication_entry,
            roster: Roster::default(),
            activation: None,
            static_tasks: Vec::new(),
            pending: Vec::new(),
            catalog,
            machine,
            accounts,
            out: protocol::communication::hand::Sender::new(),
        }
    }

    // 四手都叫在**一枚** `Control` 上：`mint` / `release` / `stop` / `state`。除 `stop` 用既有
    // **复核**——复核的判据一条也不新造（名字在装配声明里吗、账里立得起吗、待放行里有它吗），
    // 因为"该不该起"是策略，本层只答"起不起得来"。

    /// **造一个 Service**（线上 `Mint` 那一问）：复核 → （没有行就立账）→ 按声明上的来源取字节
    /// → 建域产task
    /// **复核两格**：名字得在装配声明里（`PROGRAMS`——字节与 `kind` 的声明处，帧里没有镜像）
    /// 且这一行**此刻能起**——判据与 crate::system::common::life::verdict::admit_start 同一条
    /// （`NeverStarted | Dead` 才起；表里还没这一行就先立一行）。已经在跑 / 正在起的答
    /// 本手只把这一台声明上的**来源档**转交过去
    pub fn mint(&mut self, name: String) -> Result<(), Fail> {
        let program = program_of(name.as_str()).ok_or(Fail::Unknown)?;
        let method = program.name();
        // **复核那一格**：这一行此刻能不能起——判据就是 crate::system::common::life::verdict::admit_start
        // 那一条（`NeverStarted | Dead` 才起）。已经在跑 / 正在起的答 Fail::NotReady
        // （"此刻不该起"与"半路死了"在本端是同一个下一步）。
        match self.table.find(method) {
            Some(row) => {
                if !matches!(row.state, State::NeverStarted | State::Dead) {
                    return Err(Fail::NotReady);
                }
            }
            None => {
                self.enlist(program).map_err(|_| Fail::Unknown)?;
            }
        }
        // Reserve the pending row before minting a paused Task; failure must not orphan it.
        self.pending.try_reserve(1).map_err(|_| Fail::Full)?;
        let service = match self.spawn(program) {
            Ok(service) => service,
            Err(Error::Missing | Error::Step(_)) => return Err(Fail::BadImage),
            Err(Error::Spawn) => return Err(Fail::Full),
            Err(_) => return Err(Fail::Unknown),
        };
        self.pending.push(Pending {
            name: program.name(),
            service,
        });
        Ok(())
    }

    /// **放行一枚已经造好的 Service**（线上 `Start` 那一问）：认领通道 → 放行等就绪 → 递单
    pub fn release(
        &mut self,
        name: String,
        requester: TaskId,
        mut progress: impl FnMut(&Control) -> Result<(), &'static str>,
    ) -> Result<Service, Fail> {
        let at = self
            .pending
            .iter()
            .position(|p| p.name == name.as_str())
            .ok_or(Fail::NotReady)?;
        let mut pending = self.pending.remove(at);
        let program = program_of(pending.name).ok_or(Fail::Unknown)?;
        let released = (|| {
            self.roster.inherit(pending.service.0, requester).map_err(|_| Fail::NotReady)?;
            progress(self).map_err(|_| Fail::NotReady)?;
            assemble::connect_all(program, &mut pending.service).map_err(|_| Fail::NotReady)?;
            let method = pending.name.to_string();
            self.launch(program, method.clone(), &mut pending.service)
                .map_err(|_| Fail::NotReady)?;
            self.ready(method, &mut pending.service, program.supply(), &mut progress)
                .map_err(|_| Fail::NotReady)
        })();
        if let Err(fail) = released {
            self.discard(pending.name, pending.service.0);
            let _ = progress(self);
            return Err(fail);
        }
        Ok(pending.service)
    }

    pub(crate) fn authorize_static(
        &mut self,
        task: TaskId,
    ) -> Result<(), &'static str> {
        self.static_tasks.try_reserve(1).map_err(|_| "static identity capacity")?;
        self.roster.authorize(task)?;
        self.static_tasks.push(task);
        Ok(())
    }

    /// **这一条此刻处于哪个生命阶段**（线上 `State` 那一问）
    /// **只读表里那一格**：实例坐标是另一件事（"起过、现在死了"时它仍在）——见协议那一节
    pub fn state(&self, name: String) -> Result<State, Fail> {
        if matches!(name.as_str(), "operator" | "identity") {
            let task = self.task(&name).ok_or(Fail::Unknown)?;
            if runtime::env::unit::join(task, Wait::POLL).unwrap_or(true) { return Ok(State::Dead); }
            return Ok(match self.status.phase.load(::core::sync::atomic::Ordering::Acquire) {
                phase if phase == super::Phase::Starting as u8 => State::Starting,
                phase if phase == super::Phase::Running as u8 => State::Ready,
                _ => State::Stopping,
            });
        }
        self.table
            .find(name.as_str())
            .map(|s| s.state)
            .ok_or(Fail::Unknown)
    }

    /// **起一个 Service**：按名字取那一段字节 → 建域 → 产task → 备通道账
    /// 前置：这一行**已经登记过**（Control::enlist）——没登记过由 `admit_start` 拦下
    /// 特权级仍从清单那一条取（"唯一声明处是装配表"，打包时写进去），而"字节在哪儿"本层不问
    pub fn spawn(&mut self, program: &UnitFile) -> Result<Service, Error> {
        let name = program.name().to_string();
        let entry = self.catalog.find(name.as_str()).ok_or(Error::Missing)?;
        let Some(image) = Source::initrd(self.catalog).image(name.clone()) else {
            return Err(Error::Missing);
        };
        let task = service::mint(&mut self.table, name.as_str(), image, entry.kind)
            .map_err(|_| Error::Spawn)?;
        let injected = self.publication_entry.ok_or("publication entry").and_then(|entry|
            runtime::core::res::port::ship(&runtime::env::mail::HolePie::from_token(entry), task,
                runtime::core::res::port::Access::STORE, runtime::core::res::port::Policy::NONE)
                .map(|_| ()).map_err(|_| "publication inject"));
        if let Err(why) = injected {
            self.discard(program.name(), task);
            return Err(Error::Step(why));
        }
        Ok((task, Vec::new()))
    }

    /// **放行**（**不等就绪**）：门闩、通道都在放行前定下（"两相之间的窗口就是它一步都还没跑"）
    pub fn start(&mut self, name: &str, service: &mut Service) -> Result<(), Error> {
        let (task, channels) = service;
        service::start(
            &mut self.table,
            name,
            *task,
            &[],
            channels.as_mut_slice(),
            &[],
            Wait::POLL,
        )
        .map_err(|fail| {
            let why = match fail {
                Fail::Unknown => "unknown",
                Fail::BadImage => "bad image",
                Fail::Full => "full",
                Fail::NotReady => "not ready",
            };
            protocol::debug::put(&alloc::format!("system: start failed {name} why={why}"));
            Error::Step("start failed")
        })
    }

    /// **等就绪**：`setup` 里那几条通道逐条认齐（记号即通道名）——**两条都认齐**才算起来
    /// **`Machine` 那两条的意义不同**（见 Setup::Machine）：第一条（收物料）说明"它开始跑了"
    /// 第二条（"我起完了"）说明"**它答得了了**"——而后者才是后面那几台要等的
    pub fn ready(
        &mut self,
        name: String,
        service: &mut Service,
        setup: &'static [Setup],
        mut progress: impl FnMut(&Control) -> Result<(), &'static str>,
    ) -> Result<(), Error> {
        let mut marks: Vec<Mark> = Vec::new();
        for s in setup {
            for ch in [Some(s.channel()), s.ready()].into_iter().flatten() {
                marks
                    .try_reserve(1)
                    .map_err(|_| Error::Step("no room for marks"))?;
                marks.push(Mark::of(ch));
            }
        }
        let until = runtime::env::chrono::clock().saturating_add(BOOT_MS as u64 * 1_000_000);
        let fail = loop {
            progress(self).map_err(Error::Step)?;
            self.activate_hub();
            match service::ready(
                &mut self.table, name.as_str(), service.1.as_mut_slice(), &marks, Wait::POLL,
            ) {
                Ok(already) if already
                    || self.table.find(name.as_str()).is_some_and(|row| row.state == State::Ready) =>
                    return Ok(()),
                Ok(_) if runtime::env::chrono::clock() < until => {
                    runtime::env::room::sleep(Duration::from_millis(RETRY_MS as u64))
                        .map_err(|_| Error::Step("ready wait"))?;
                }
                Ok(_) => break Fail::NotReady,
                Err(fail) => break fail,
            }
        };
        let why = match fail {
            Fail::Unknown => "unknown",
            Fail::BadImage => "bad image",
            Fail::Full => "full",
            Fail::NotReady => "not ready",
        };
        protocol::debug::put(&alloc::format!(
            "system: not ready {name} why={why} marks={}", marks.len(),
        ));
        Err(Error::Step("start failed"))
    }

    pub fn stop(&mut self, name: String) -> Result<(), Fail> {
        if matches!(name.as_str(), "operator" | "identity") { return Err(Fail::Unknown); }
        let task = self.task(name.as_str()).ok_or(Fail::Unknown)?;
        service::stop(&mut self.table, name.as_str())?;
        if name == "hub" {
            self.activation = None;
        }
        self.roster.unbind(task).map_err(|_| Fail::NotReady)
    }

    pub(crate) fn task(&self, name: &str) -> Option<TaskId> {
        use ::core::sync::atomic::Ordering;
        match name {
            "operator" => return Some(TaskId::new(self.status.operator.load(Ordering::Acquire))),
            "identity" => return Some(TaskId::new(self.status.identity.load(Ordering::Acquire))),
            _ => {}
        }
        match self.table.find(name)?.slot {
            Slot::Live { task, .. } => Some(task),
            Slot::None => None,
        }
    }

    pub(crate) fn tasks(&self) -> impl Iterator<Item = TaskId> + '_ {
        self.table.living().filter_map(|row| match row.slot {
            Slot::Live { task, .. } if matches!(row.state, State::Starting | State::Ready) => Some(task),
            _ => None,
        })
    }

    pub(crate) fn activate_hub(&self) {
        if let Some(activation) = &self.activation {
            activation.poll(self);
        }
    }

    /// Compensate a failed launch, including a successful bind followed by IPC/setup failure.
    pub(crate) fn discard(&mut self, name: &str, task: TaskId) {
        if name == "hub" {
            self.activation = None;
        }
        let _ = runtime::env::room::doom(task);
        if let Some(row) = self.table.find(name) {
            if let Slot::Live { team: Some(team), .. } = row.slot {
                let _ = runtime::env::unit::oust(team);
            }
        }
        if let Err(why) = self.roster.unbind(task) {
            protocol::debug::put(&alloc::format!("system: compensation {name}: {why}"));
        }
        self.table.detach(name);
        self.table.set_state(name, State::Dead);
        self.static_tasks.retain(|known| *known != task);
    }

    /// **该收了**：账上活着的都是常驻台——会走的都走了、听令的已经发过话
    pub fn due(&self) -> bool {
        core::due(&self.table)
    }

    /// **收讫了**：账上一个不剩。与内核 `conductor::done()`（`PUSHED == REAPED`）同名同形
    pub fn done(&self) -> bool {
        core::done(&self.table)
    }

    pub fn await_ready(&self, name: &str, wait: Wait) -> Result<(), Fail> {
        if matches!(name, "operator" | "identity") {
            return if self.status.phase.load(::core::sync::atomic::Ordering::Acquire) == super::Phase::Running as u8 {
                Ok(())
            } else { Err(Fail::NotReady) };
        }
        let mut left = match wait {
            Wait::POLL => 0,
            Wait::AtMost(ms) => ms,
            Wait::Forever => READY_MS,
        };
        loop {
            match self.table.find(name).map(|s| s.state) {
                Some(State::Ready | State::Stopping | State::Dead) => return Ok(()),
                Some(State::NeverStarted | State::Starting) => {}
                None => return Err(Fail::Unknown),
            }
            if left == 0 {
                return Err(Fail::NotReady);
            }
            let _ = runtime::env::room::sleep(Duration::from_millis(RETRY_MS as u64));
            left -= 1;
        }
    }

    pub fn stop_rest(&mut self) {
        // 先把名字抄下来再动表（表是定长的、行数有上界——与 `sweep` 同一个形状）。
        let mut names = [const { String::new() }; Table::CAP];
        let mut n = 0usize;
        for row in self.table.living() {
            if !matches!(row.state, State::Starting | State::Ready) {
                continue;
            }
            if matches!(row.slot, Slot::Live { team: None, .. }) {
                continue;
            }
            names[n] = row.name.clone();
            n += 1;
        }
        for name in &names[..n] {
            let _ = self.stop(name.clone());
        }
    }
}

/// **装一条通道**（放行前）：铸本端那一枚（刻 `ch` 的记号）交给这条服务的域、并顺手试认它那一枚
/// （`POLL` = 不等：放行前它一步都还没跑，认不到是常态）——放行后按同一个记号再认一次
/// （service::ready 逐条 `claim`）
/// 只碰通道、不碰 `Control` 的任何一格，故是自由函数（Control::connect 那一层是白加的壳）
pub fn connect(to: TaskId, ch: &'static str) -> Result<Endpoint, Error> {
    establish::endpoint(to, Mark::of(ch), Wait::POLL).map_err(|_| Error::Step("connect failed"))
}

/// **名字 → 装配声明**：边上那四手要的那一份（`kind` / `setup` 都在它那里）
/// 找不到 ⇒ `None`：这个名字不在这一景的装配声明里（`PROGRAMS` 是唯一声明处；帧里没有镜像
/// 故"起哪一台"这件事只认本表）
/// **只认"由编排域起"的那几台**（`relation.after.is_some()`）：`root` / `system` 自己不在
/// 那张单里——运行期再造一枚"机器本身"不是本协议的意思。判据与 crate::system::run::scene
/// 的过滤同一句
fn program_of(name: &str) -> Option<&'static UnitFile> {
    PROGRAMS
        .iter()
        .copied()
        .find(|p| p.relation.after.is_some() && p.name() == name)
}
