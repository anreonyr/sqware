//! 它只答一件事：这一条服务在不在、怎么被创建 / 配置 / 启动 / 停止。
//! 四套协议的语义（命名 / 身份 / 横向关系 / 存在信号）**不在这一层**：那些是程序声明上的
//! 各自的手上。
//! **`Service` 不另立类型**：它就是"一枚线程 ＋ 它那几条通道"（Service 是那两样的别名）。

use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;

use ::core::time::Duration;

use crate::system::common::life::table::{Slot, State, Table};
use crate::system::common::life::verdict::Fail;
use env::{Mark, TaskId, Wait};
use protocol::communication::session::establish::{self, Endpoint};

use crate::boot::{Accounts, Catalog};
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
    /// 身子没产出来（建域 / 产线程那一关，或账里没有这一行）
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

/// **一条运行时服务**：一枚线程 ＋ 它那几条通道（会话）
/// （第一条是 `records`），随后板 / 树两条装配路也各往这本账里添一件
/// **一条通道一件持有者**（Endpoint）——`Endpoint` 只装两枚孔，故"一条关系 N 条通道"那一档
/// 名字不进这个别名——账里那一行就是名字，调用方手里也有 `UnitFile`
pub type Service = (TaskId, Vec<Endpoint>);

/// **Service 的生命周期与装配环境**
pub struct Control {
    table: Table,
    pending: Vec<Pending>,
    catalog: Catalog<'static>,
    machine: Machine,
    accounts: Accounts,
    /// **上一手入册那一单的写端**（Setup::Machine 那一格）
    out: protocol::communication::hand::Sender<protocol::service::hub::Enroll>,
}

/// 一枚**已造未放行**的身子（Control::pending 那一格）
struct Pending {
    /// 装配表上的名字（`&'static str`——`spawn` / `start` 要它）
    name: &'static str,
    /// 那一枚线程 ＋ 它已经认领的通道
    service: Service,
}

impl Control {
    /// 就位（四样都由 `System` 在引导之后交进来）
    pub fn new(catalog: Catalog<'static>, machine: Machine, accounts: Accounts) -> Control {
        Control {
            table: Table::new(),
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
    /// → 建域产线程
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
        let service = match self.spawn(program) {
            Ok(service) => service,
            Err(Error::Missing | Error::Step(_)) => return Err(Fail::BadImage),
            Err(Error::Spawn) => return Err(Fail::Full),
            Err(_) => return Err(Fail::Unknown),
        };
        self.pending.try_reserve(1).map_err(|_| Fail::Full)?;
        self.pending.push(Pending {
            name: program.name(),
            service,
        });
        Ok(())
    }

    /// **放行一枚已经造好的 Service**（线上 `Start` 那一问）：认领通道 → 放行等就绪 → 递单
    pub fn release(&mut self, name: String) -> Result<Service, Fail> {
        let at = self
            .pending
            .iter()
            .position(|p| p.name == name.as_str())
            .ok_or(Fail::NotReady)?;
        let mut pending = self.pending.remove(at);
        let program = program_of(pending.name).ok_or(Fail::Unknown)?;
        assemble::connect_all(program, &mut pending.service).map_err(|_| Fail::NotReady)?;
        let method = pending.name.to_string();
        // 放行 + 递单（次序是硬的：物料要落到它交回的那条路上）。
        self.launch(program, method.clone(), &mut pending.service)
            .map_err(|_| Fail::NotReady)?;
        // **再等就绪**（线上这条路上没有挂板 / 挂树那两手——那两件是装配期的事，
        // 见 crate::system::control::enroll 里 `launch` 那一格的注）。
        self.ready(method, &mut pending.service, program.supply())
            .map_err(|_| Fail::NotReady)?;
        Ok(pending.service)
    }

    /// **这一条此刻处于哪个生命阶段**（线上 `State` 那一问）
    /// **只读表里那一格**：实例坐标是另一件事（"起过、现在死了"时它仍在）——见协议那一节
    pub fn state(&self, name: String) -> Result<State, Fail> {
        self.table
            .find(name.as_str())
            .map(|s| s.state)
            .ok_or(Fail::Unknown)
    }

    /// **起一个 Service**：按名字取那一段字节 → 建域 → 产线程 → 备通道账
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
        service::ready(
            &mut self.table,
            name.as_str(),
            service.1.as_mut_slice(),
            &marks,
            Wait::AtMost(BOOT_MS),
        )
        .map(|_| ())
        .map_err(|fail| {
            let why = match fail {
                Fail::Unknown => "unknown",
                Fail::BadImage => "bad image",
                Fail::Full => "full",
                Fail::NotReady => "not ready",
            };
            protocol::debug::put(&alloc::format!(
                "system: not ready {name} why={why} marks={}",
                marks.len()
            ));
            Error::Step("start failed")
        })
    }

    pub fn stop(&mut self, name: String) -> Result<(), Fail> {
        service::stop(&mut self.table, name.as_str())
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
            let _ = service::stop(&mut self.table, name.as_str());
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
