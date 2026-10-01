//! system::control — **Service 的生命周期**：建 / 配 / 起 / 停。
//! 它只答一件事：**这一条服务在不在、怎么被创建 / 配置 / 启动 / 停止。**
//! 四套协议的语义（命名 / 身份 / 横向关系 / 存在信号）**不在这一层**：那些是程序声明上的
//! **边**，由 [`Assembly::assemble`](crate::system::Assembly) 在装配那一趟里按次序落到四轴
//! 各自的手上。
//! ```text
//!   UnitFile（静态声明）── spawn → connect → start → wire ──▶ Service（域 + 线程 + 通道）
//! ```
//! **`Service` 不另立类型**：它就是"一枚线程 ＋ 它那几条通道"（[`Service`] 是那两样的别名）。

use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;

use ::core::time::Duration;

use crate::system::control::core::Fail;
use crate::system::control::desk::{Slot, State, Table};
use env::{Mark, TaskId, Wait};
use protocol::communication::establish::{self, Endpoint};

use crate::boot::{Accounts, Catalog};
use crate::system::machine::Machine;
use crate::system::source::Source;
use crate::unit::{PROGRAMS, Setup, UnitFile};

pub mod assemble;
pub mod core;
pub mod desk;
pub mod service;
pub mod supervise;

/// 装配失败的编号——定义见 [`crate::unit::Died`]（本处只是转发）。
pub use crate::unit::Died;

/// 认身份门牌的短等间隔（毫秒）：门牌由名册起手交出，装配者这一侧只是短等。
pub const RETRY_MS: usize = 1;

/// 等子域就绪/交通道的上限（毫秒）。**必须有界**：子域要是死在头几步，本域不能陪着挂死。
pub const READY_MS: usize = 1000;

/// **等"它起完了"那一条通道的上限**（毫秒）——比 [`READY_MS`] 宽得多。
pub const BOOT_MS: usize = 5000;

/// 装配失败的编号（通用的那几个；按服务分的编号住在装配表旁边）。
pub const E_MANIFEST: Died = 2;
pub const E_PROGRAM: Died = 3;
pub const E_TABLE: Died = 4;

/// **Control 的失败域**：一格 = 死在装配的哪一类。
/// **不是全局错误表**：它是装配那一圈的返回类型，号（`env::Reason`）由调用方在边界上折
/// ——按服务分的号归装配表那一格 `died`。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
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
/// 线程与通道都在这里：`spawn` 只挂线程（账是空的），放行前 `connect` 按 `setup` 逐条装
/// （第一条是 `records`），随后板 / 树两条装配路也各往这本账里添一件。
/// **一条通道一件持有者**（[`Endpoint`]）——`Endpoint` 只装两枚孔，故"一条关系 N 条通道"那一档
/// 在这里就是**几个 `Endpoint`**，不是一个能装的容器类型。**这本账归装配者拿着**：那几枚孔是
/// 本域铸出去、客人将来要认的那一半，放早了客人就没得认（见 [`Assembly::assemble`](crate::system::Assembly::assemble)）。
/// 名字不进这个别名——账里那一行就是名字，调用方手里也有 `UnitFile`。
pub type Service = (TaskId, Vec<Endpoint>);

/// **Service 的生命周期与装配环境**。
pub struct Control {
    table: Table,
    pending: Vec<Pending>,
    catalog: Catalog<'static>,
    machine: Machine,
    /// **boot 的两块账**：发货那一趟按坐标取源（全机门闩在本域表里）。
    /// **照实记（并域那一刀）**：这一格从前是 `boot: Endpoint`——一条跨域问答路的凭据
    /// （`establish::endpoint` ＋ 那条记号 ＋ 一问一答两只缓冲）。门闩并到本域之后，
    /// 取源就是按坐标查这张账。
    accounts: Accounts,
    /// **上一手入册那一单的写端**（`Setup::Machine` 那一格）。
    /// **为什么住在这里**：那一单是**递完就完**的（没有回话），而递出去的字节要活到对面取走
    /// ——所以它必须住在比调用帧更长的地方。装配者正是比它长的那一位：一本 `Control` 活到
    /// 装配完。下一台入册前先收口那一手在 `send` 里（那一刻上一台必然已经取走了）。
    out: protocol::communication::sender::Sender<protocol::driver::hub::Enroll>,
}

/// 一枚**已造未放行**的身子（[`Control::pending`] 那一格）。
struct Pending {
    /// 装配表上的名字（`&'static str`——`spawn` / `start` 要它）。
    name: &'static str,
    /// 那一枚线程 ＋ 它已经认领的通道。
    service: Service,
}

impl Control {
    /// 就位（四样都由 `System` 在引导之后交进来）。
    pub fn new(catalog: Catalog<'static>, machine: Machine, accounts: Accounts) -> Control {
        Control {
            table: Table::new(),
            pending: Vec::new(),
            catalog,
            machine,
            accounts,
            out: protocol::communication::sender::Sender::new(),
        }
    }

    // 四手都叫在**一枚** `Control` 上：`mint` / `release` / `stop` / `state`。除 `stop` 用既有
    // 那一手（`service::stop`：下令即回、状态到 `Stopping`）外，其余三手只在这里加一层
    // **复核**——复核的判据一条也不新造（名字在装配声明里吗、账里立得起吗、待放行里有它吗），
    // 因为"该不该起"是策略，本层只答"起不起得来"。

    /// **造一个 Service**（线上 `Mint` 那一问）：复核 → （没有行就立账）→ 按声明上的来源取字节
    /// → 建域产线程。
    /// **复核两格**：名字得在装配声明里（`PROGRAMS`——字节与 `kind` 的声明处，帧里没有镜像），
    /// 且这一行**此刻能起**——判据与 [`crate::system::control::core::admit_start`] 同一条
    /// （`NeverStarted | Dead` 才起；表里还没这一行就先立一行）。已经在跑 / 正在起的答
    /// [`Fail::NotReady`]。造出来**恒为未放行**（`service::mint` 的口径），身子收进
    /// [`Control::pending`] 等 [`Control::release`]。
    /// **它不碰镜像**：取字节那一面（[`crate::system::source`]）是 `spawn` 的唯一消费者，
    /// 本手只把这一台声明上的**来源档**转交过去。
    pub fn mint(&mut self, name: String) -> Result<(), Fail> {
        let program = program_of(name.as_str()).ok_or(Fail::Unknown)?;
        let method = program.name();
        // **复核那一格**：这一行此刻能不能起——判据就是 [`crate::system::control::core::admit_start`]
        // 那一条（`NeverStarted | Dead` 才起）。已经在跑 / 正在起的答 [`Fail::NotReady`]
        // （"此刻不该起"与"半路死了"在本端是同一个下一步）。
        match self.table.find(method) {
            Some(row) => {
                if !matches!(row.state, State::NeverStarted | State::Dead) {
                    return Err(Fail::NotReady);
                }
            }
            // 表里还没有这一行：立一行（与装配那一趟同一手）。
            None => {
                self.enlist(program).map_err(|_| Fail::Unknown)?;
            }
        }
        let service = match self.spawn(program) {
            Ok(service) => service,
            // 清单里没有这一台 / 来源那一档取不到字节：都是"这一段字节没有"。
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

    /// **放行一枚已经造好的 Service**（线上 `Start` 那一问）：认领通道 → 放行等就绪 → 递单。
    /// 次序与装配那一趟逐字同源（`assemble::connect_all` / [`Control::launch`]）：**通道在放行
    pub fn release(&mut self, name: String) -> Result<Service, Fail> {
        let at = self
            .pending
            .iter()
            .position(|p| p.name == name.as_str())
            .ok_or(Fail::NotReady)?;
        let mut pending = self.pending.remove(at);
        let program = program_of(pending.name).ok_or(Fail::Unknown)?;
        // 通道：放行前逐条装（记号 = 通道名，放行后逐条认领——与装配那一趟**同一手**）。
        assemble::connect_all(program, &mut pending.service).map_err(|_| Fail::NotReady)?;
        let method = pending.name.to_string();
        // 放行 + 递单（次序是硬的：物料要落到它交回的那条路上）。
        self.launch(program, method.clone(), &mut pending.service)
            .map_err(|_| Fail::NotReady)?;
        // **再等就绪**（线上这条路上没有挂板 / 挂树那两手——那两件是装配期的事，
        // 见 [`crate::system::control::assemble`] 里 `launch` 那一格的注）。
        self.ready(method, &mut pending.service, program.supply())
            .map_err(|_| Fail::NotReady)?;
        Ok(pending.service)
    }

    /// **这一条此刻处于哪个生命阶段**（线上 `State` 那一问）。
    /// **只读表里那一格**：实例坐标是另一件事（"起过、现在死了"时它仍在）——见协议那一节。
    pub fn state(&self, name: String) -> Result<State, Fail> {
        self.table
            .find(name.as_str())
            .map(|s| s.state)
            .ok_or(Fail::Unknown)
    }

    /// **起一个 Service**：按名字取那一段字节 → 建域 → 产线程 → 备通道账。
    /// 前置：这一行**已经登记过**（[`Control::enlist`]）——没登记过由 `admit_start` 拦下。
    /// **那一段字节从哪儿来**：由这一台声明上的**来源档**（`origin`）定，取字节那一面住
    /// [`crate::system::source`]——**这里就是那一面的唯一消费者**（下面 `service::mint` 那一行）。
    /// 特权级仍从清单那一条取（"唯一声明处是装配表"，打包时写进去），而"字节在哪儿"本层不问。
    pub fn spawn(&mut self, program: &UnitFile) -> Result<Service, Error> {
        let name = program.name().to_string();
        let entry = self.catalog.find(name.as_str()).ok_or(Error::Missing)?;
        // **取字节那一面的唯一消费者**：这一景那本账（`self.catalog`）按名字给那一段 `&[u8]`。
        let Some(image) = Source::initrd(self.catalog).image(name.clone()) else {
            return Err(Error::Missing);
        };
        let task = service::mint(&mut self.table, name.as_str(), image, entry.kind)
            .map_err(|_| Error::Spawn)?;
        Ok((task, Vec::new()))
    }

    /// **放行**（**不等就绪**）：门闩、通道都在放行前定下（"两相之间的窗口就是它一步都还没跑"）。
    pub fn start(&mut self, name: &str, service: &mut Service) -> Result<(), Error> {
        let (task, channels) = service;
        // `marks` 空 ⇒ 这一手**只放行**（`service::start` 那条"还活着、只是没宣布"的分支）。
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

    /// **等就绪**：`setup` 里那几条通道逐条认齐（记号即通道名）——**两条都认齐**才算起来。
    /// **`Machine` 那两条的意义不同**（见 [`Setup::Machine`]）：第一条（收物料）说明"它开始跑了"，
    /// 第二条（"我起完了"）说明"**它答得了了**"——而后者才是后面那几台要等的。
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

    /// **该收了**：账上活着的都是常驻台——会走的都走了、听令的已经发过话。
    pub fn due(&self) -> bool {
        core::due(&self.table)
    }

    /// **收讫了**：账上一个不剩。与内核 `conductor::done()`（`PUSHED == REAPED`）同名同形。
    pub fn done(&self) -> bool {
        core::done(&self.table)
    }

    pub fn await_ready(&self, name: &str, wait: Wait) -> Result<(), Fail> {
        let mut left = match wait {
            Wait::POLL => 0,
            Wait::AtMost(ms) => ms,
            // **不认"永远"**：这一问是装配那一趟里的一格，拿 `READY_MS` 当上限。
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
            // **本域那一枚不在这里收**：它的"域"就是本域，收它就是扑杀本域自己——它随本域
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
/// （[`service::ready`] 逐条 `claim`）。
/// 只碰通道、不碰 `Control` 的任何一格，故是自由函数（`Control::connect` 那一层是白加的壳）。
pub fn connect(to: TaskId, ch: &'static str) -> Result<Endpoint, Error> {
    establish::endpoint(to, Mark::of(ch), Wait::POLL).map_err(|_| Error::Step("connect failed"))
}

/// **名字 → 装配声明**：边上那四手要的那一份（`kind` / `setup` 都在它那里）。
/// 找不到 ⇒ `None`：这个名字不在这一景的装配声明里（`PROGRAMS` 是唯一声明处；帧里没有镜像，
/// 故"起哪一台"这件事只认本表）。
/// **只认"由编排域起"的那几台**（`relation.after.is_some()`）：`root` / `system` 自己不在
/// 那张单里——运行期再造一枚"机器本身"不是本协议的意思。判据与 [`crate::system::assemble`]
/// 的过滤同一句。
fn program_of(name: &str) -> Option<&'static UnitFile> {
    PROGRAMS
        .iter()
        .copied()
        .find(|p| p.relation.after.is_some() && p.name() == name)
}
