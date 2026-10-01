//! system::control — **Service 的生命周期**：建 / 配 / 起 / 停。
//!
//! 它只答一件事：**这一条服务在不在、怎么被创建 / 配置 / 启动 / 停止。**
//! 四套协议的语义（命名 / 身份 / 横向关系 / 存在信号）**不在这一层**：那些是程序声明上的
//! **边**，由 [`Assembly::assemble`](crate::system::Assembly) 在装配那一趟里按次序落到四轴
//! 各自的手上。
//!
//! ```text
//!   Program（静态声明）── spawn → connect → start → wire ──▶ Service（域 + 线程 + 通道）
//! ```
//!
//! **`Service` 不另立类型**：它就是"一枚线程 ＋ 它那几条通道"（[`Service`] 是那两样的别名）。
//! 原先那个六格结构体（`name` / `task` / `quay` / `marks` / `needs` / `channel`）是"把一个
//! 函数拆成三个调用点"逼出来的壳——后三格每次都能从 `setup` 现推，`name`/`task` 账里本来就有。
//!
//! 手里只有装配环境四样：账（[`Table`]）、清单（[`Catalog`]）、机器自述、与引导域的会话。
//! 存在信号那本账（道表与那只组）住 [`supervise`] 那一相。
//!
//! **`mint` 到 `start` 之间那一枚身子住 [`Control::pending`]**（照实记）：协议那四手在运行期是
//! **两问**（`mint` / `start`），两问各是一趟消息 ⇒ "已造未放行"必须有个安放处。
//!
//! **内核那一侧的手住 [`service`]**；**立账与递单住 [`assemble`]**；**监督相住 [`supervise`]**；
//! **挂上树那三件住 [`mount`]**（铸入口 / 递出去；"落"由持树者做）。
//!
//! # 这一面今天外面也到得了（照实记：谁把那一格落上树，换过三次）
//!
//! [`protocol::system::control::Face`] 的形状是定稿的（四手 ＋ 帧 ＋ 记号）。task-4 把它挂上树
//! 那条路**撤过一次**——那时挂树要一枚**一次性**边沿线程去落门牌，而它一收尾，持树者表里那枚
//! 入口副本会被内核的派生链级联摘掉（`cull` 沿 `sire` **跨任务**摘后代），plate 那一格于是留着
//! 一个**取不回的号**：查得到、`find` 拿不回来。证据链三处
//! （`kernel/src/work/unit/gate/accord.rs`／`kernel/src/boot.rs` 的 `EXIT_HOOKS`／
//! `kernel/src/work/unit/gate/cull.rs::doom`）与"**前置 = 铸入口那一枚线程必须长命**"写在
//! [`crate::system::Assembly::supervise`] 的照实记里。
//!
//! **今天那一格满足了，而且没有第三方上树**：铸入口的是编排域主线程（它此后就进监督那一趟，
//! **本域活多久它活多久**），而"把这一格落到 `/svc/sys/control`"由**持树者在自己核里做**
//! （[`mount::entry`] 铸那一枚 → [`crate::system::operator::bridge::Tree::plate`] 递过去 →
//! 持树者 `part` ＋ `land`）。于是 `/svc/sys/control` 与名册 / 盟册那两族那**四格**逐字同形：
//! 任何走到树的任务 `operator::Face::tile` 一查就有，
//! [`protocol::system::control::Face::of`] 直接成立——那位真客人是 `harness/src/probe_control.rs`。
//!
//! **代价照实说**：挂上之后，**任何已绑身份**的域都能按 `Permit::Unset` 取回那一枚入口，进而
//! `mint` / `start` / `stop` 装配表里的任意一台程序。这不是新开的口子——`doom`（收掉一个域）
//! 今天同样没有门禁；若将来要收，收的地方是那一格的 `Permit`（协议那一侧改一格，见
//! `protocol::system::operator::Permit`），不是这一层。

use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;

use ::core::time::Duration;

use crate::system::control::core::Fail;
use crate::system::control::desk::{Slot, State, Table};
use env::manifest;
use env::{Mark, TaskId, Wait};
use protocol::communication::establish::{self, Endpoint};

use crate::program::{PROGRAMS, Program, Setup};
use crate::root::boot;
use crate::system::machine::Machine;
use crate::system::source::{self, Source};

pub mod assemble;
pub mod core;
pub mod desk;
pub mod service;
pub mod supervise;

/// 装配失败的编号——定义见 [`crate::program::Died`]（本处只是转发）。
pub use crate::program::Died;

/// 认身份门牌的短等间隔（毫秒）：门牌由名册起手交出，装配者这一侧只是短等。
pub const RETRY_MS: usize = 1;

/// 等子域就绪/交通道的上限（毫秒）。**必须有界**：子域要是死在头几步，本域不能陪着挂死。
pub const READY_MS: usize = 1000;

/// **等"它起完了"那一条通道的上限**（毫秒）——比 [`READY_MS`] 宽得多。
///
/// **照实记（这个数是量出来的）**：`Setup::Machine` 那一格把"我交回了一枚孔"与"我答得了了"
/// 拆成两条通道之后，等后面那一条要等的是**一台机器最慢的一次起手**：读一遍设备树、逐类立盟
/// （每类两趟盟册）、逐类逐台落格、**每格查回来验一遍**（`bridge::land` 那条口径）。实测
/// （debug 档、qemu `-smp 4`）：那一整段落在 **2.5 s 上下**——[`READY_MS`] 那一秒装不下，
/// 于是装配者会在它落完之前就判"没起来"。
///
/// **它仍然有界**：那是 [`READY_MS`] 那一句的全部意义（子域死在头几步时本域不能陪着挂死）。
/// 只等那**一条**通道用这个宽额度；其余几条（通道 / 板 / 树）照旧 [`READY_MS`]。
pub const BOOT_MS: usize = 5000;

/// 装配失败的编号（通用的那几个；按服务分的编号住在装配表旁边）。
pub const E_MANIFEST: Died = 2;
pub const E_PROGRAM: Died = 3;
pub const E_TABLE: Died = 4;

/// **Control 的失败域**：一格 = 死在装配的哪一类。
///
/// **不是全局错误表**：它是装配那一圈的返回类型，号（`env::Reason`）由调用方在边界上折
/// ——按服务分的号归装配表那一格 `died`。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// 名字读不懂（旧 `env::wire::name` 那一关已退场：构造面不判，今天仓内没有生产者）。
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
/// 本域铸出去、客人将来要认的那一半，放早了客人就没得认（见 [`Assembly::assemble`](crate::system::Assembly::assemble)）。
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
    /// **已造未放行的那几枚身子**（`Mint` 之后、`Start` 之前）。
    ///
    /// **照实记（这一格为什么在）**：线上那四手是 **mint → start** 两问，而 `Service`
    /// （线程 ＋ 它的通道账）在那两问之间必须有处安放——装配那一趟把它拿在自己栈上就完事，
    /// 运行期这两问却是**两趟消息**。键取装配表上的名字（`&'static str`）：
    /// `Control::spawn` 要的正是它，而它也是这一台在 `PROGRAMS` 里的唯一坐标。
    pending: Vec<Pending>,
    catalog: Catalog<'static>,
    machine: Machine,
    boot: Endpoint,
    /// **上一手入册那一单的写端**（`Setup::Machine` 那一格）。
    ///
    /// **为什么住在这里**：那一单是**递完就完**的（没有回话），而递出去的字节要活到对面取走
    /// ——所以它必须住在比调用帧更长的地方。装配者正是比它长的那一位：一本 `Control` 活到
    /// 装配完。下一台入册前先收口那一手在 `send` 里（那一刻上一台必然已经取走了）。
    ///
    /// **照实记（`Outbox` 并进 `Sender`）**：从前这一格只是"缓冲 ＋ 那只手"，孔另由
    /// `Sender::from_token(tx)` 每一趟现给 ⇒ 同一个孔上站着两个写端（一个持孔、一个持字节），
    /// "这一段字节活在谁手里"两处都能答。今天写端只有一枚：孔、字节、那只手都在它身上。
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
    pub fn new(catalog: Catalog<'static>, machine: Machine, boot: Endpoint) -> Control {
        Control {
            table: Table::new(),
            pending: Vec::new(),
            catalog,
            machine,
            boot,
            out: protocol::communication::sender::Sender::new(),
        }
    }

    // ── 边上那四手（`protocol::system::control::Face` 的实现体）───────────
    //
    // 四手都叫在**一枚** `Control` 上：`mint` / `release` / `stop` / `state`。除 `stop` 用既有
    // 那一手（`service::stop`：下令即回、状态到 `Stopping`）外，其余三手只在这里加一层
    // **复核**——复核的判据一条也不新造（名字在装配声明里吗、账里立得起吗、待放行里有它吗），
    // 因为"该不该起"是策略，本层只答"起不起得来"。

    /// **造一个 Service**（线上 `Mint` 那一问）：复核 → （没有行就立账）→ 按声明上的来源取字节
    /// → 建域产线程。
    ///
    /// **复核两格**：名字得在装配声明里（`PROGRAMS`——字节与 `kind` 的声明处，帧里没有镜像），
    /// 且这一行**此刻能起**——判据与 [`crate::system::control::core::admit_start`] 同一条
    /// （`NeverStarted | Dead` 才起；表里还没这一行就先立一行）。已经在跑 / 正在起的答
    /// [`Fail::NotReady`]。造出来**恒为未放行**（`service::mint` 的口径），身子收进
    /// [`Control::pending`] 等 [`Control::release`]。
    ///
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
    ///
    /// 次序与装配那一趟逐字同源（`assemble::connect_all` / [`Control::launch`]）：**通道在放行
    /// 之前装**，配给在放行之后递——两相之间的窗口就是"它一步都还没跑"。
    ///
    /// **复核一格**：这一手只认**自己刚造的那一枚**（`pending` 里有它）；没有 ⇒ [`Fail::NotReady`]
    /// （"此刻不该起"与"半路死了"在这一格是同一句话：本端下一步相同）。
    ///
    /// **答的是那一枚身子**（照实记：这一手原先答 `()`）：`(TaskId, Vec<Endpoint>)` 里那枚
    /// `TaskId` 是**这一族唯一交得出域外的东西**——线上 `Start` 那一答第三格就是它
    /// （[`protocol::system::control::frame::said_task`]），而通道那本账留在本域（`Endpoint`
    /// 的两枚孔是"持有它的那张表里才念得动"的号，交不到客人手里，见协议那一份的照实记）。
    /// 故这一手的返回值**两头都用**：装配面拿它做后续（挂树 / 眼睛），线上那一侧只取第一格。
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
        self.ready(method, &mut pending.service, program.demand.setup)
            .map_err(|_| Fail::NotReady)?;
        Ok(pending.service)
    }

    /// **这一条此刻处于哪个生命阶段**（线上 `State` 那一问）。
    ///
    /// **只读表里那一格**：实例坐标是另一件事（"起过、现在死了"时它仍在）——见协议那一节。
    pub fn state(&self, name: String) -> Result<State, Fail> {
        self.table
            .find(name.as_str())
            .map(|s| s.state)
            .ok_or(Fail::Unknown)
    }

    /// **起一个 Service**：按名字取那一段字节 → 建域 → 产线程 → 备通道账。
    ///
    /// 前置：这一行**已经登记过**（[`Control::enlist`]）——没登记过由 `admit_start` 拦下。
    ///
    /// **那一段字节从哪儿来**：由这一台声明上的**来源档**（`origin`）定，取字节那一面住
    /// [`crate::system::source`]——**这里就是那一面的唯一消费者**（下面 `service::mint` 那一行）。
    /// 特权级仍从清单那一条取（"唯一声明处是装配表"，打包时写进去），而"字节在哪儿"本层不问。
    ///
    /// 通道账起手是空的：`setup` 里那几条由 [`connect`] 逐条装上（放行之前）。
    pub fn spawn(&mut self, program: &Program) -> Result<Service, Error> {
        let name = program.name().to_string();
        let entry = self.catalog.find(name.as_str()).ok_or(Error::Missing)?;
        // **取字节那一面的唯一消费者**：这一景那本账（`self.catalog`）按名字给那一段 `&[u8]`。
        // **声明里不问来源**（照实记：`Origin` 那一格 0 个选择者，随那本账一起退场，见
        // [`crate::program::Demand`] 底下那一段）。
        let image = Source::initrd(self.catalog)
            .image(name.clone())
            .map_err(|e| match e {
                source::Error::Missing => Error::Missing,
                // 其余失败按"那一段字节取不到"报，读数带它自己的说法。
                other => Error::Step(other.said()),
            })?;
        let task =
            service::mint(&mut self.table, name.as_str(), image, entry.kind)
                .map_err(|_| Error::Spawn)?;
        Ok((task, Vec::new()))
    }

    /// **放行**（**不等就绪**）：门闩、通道都在放行前定下（"两相之间的窗口就是它一步都还没跑"）。
    ///
    /// **为什么从"放行 ＋ 等就绪"拆成一手**（照实记）：等就绪要等的那几条通道里，`Machine`
    /// 那一格的**第一条（收物料）递在放行之后、第二条（"我起完了"）之前**——而递物料又必须
    /// 等本域放行（它第一件事就是铸那一枚孔）。三件事的次序是
    /// **放行 → 递物料 → 等就绪**，拧成一相就死锁（见 [`Control::launch`] 的那一段）。
    ///
    /// 返 `Err` = 放行那一步没成（本相失败时实例与状态如实留在表里，调用方用 [`Control::stop`]
    /// 收尾）。
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
        .map_err(|_| Error::Step("start failed"))
    }

    /// **等就绪**：`setup` 里那几条通道逐条认齐（记号即通道名）——**两条都认齐**才算起来。
    ///
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
        // **额度按这一格最宽的那一条算**（`Machine` 那两条里的"我起完了"，见 [`BOOT_MS`]）：
        // 窄的那几条早在额度之内（多等的那几毫秒只在真死时才花得出去）。
        service::ready(
            &mut self.table,
            name.as_str(),
            service.1.as_mut_slice(),
            &marks,
            Wait::AtMost(if marks.len() > 1 { BOOT_MS } else { READY_MS }),
        )
        .map(|_| ())
        .map_err(|_| Error::Step("start failed"))
    }

    /// 收掉一条 Service：**下令即回，不等它收完**。
    ///
    /// **两格语义一个字不省**：这一手只把状态推到 `Stopping`（`service::stop` 的口径）；
    /// 落 `Dead` 的是**监督那一趟**（[`supervise`] 的 `until` 两相）。
    pub fn stop(&mut self, name: String) -> Result<(), Fail> {
        service::stop(&mut self.table, name.as_str())
    }

    // **照实记（`until` / `observe` 两具读手已退场）**：它们从前是 `Control` 面上的读口
    // （"等它收尾"／"盯它一眼"），而协议那四手（`mint` / `start` / `stop` / `state`）都不需要
    // 它们——**全仓零消费者**。按"没有读者的格不留在面上"删掉两具。

    // ── 一张单（收场那一相的三手）────────────────────────────────────
    //
    // **照实记（这一列原先没有名字）**：整机什么时候收、收干净没有，原先是 `Watch::run` 里的
    // 一句"最后一位退场"＋一句裸 `return`——判据是**位次**，而且没人读账。这三手把那一列
    // 收成与 `mint` / `release` / `stop` / `state` 并列的形状：**判在 `core`，手在本层**。

    /// **该收了**：账上活着的都是常驻台——会走的都走了、听令的已经发过话。
    pub fn due(&self) -> bool {
        core::due(&self.table)
    }

    /// **收讫了**：账上一个不剩。与内核 `conductor::done()`（`PUSHED == REAPED`）同名同形。
    pub fn done(&self) -> bool {
        core::done(&self.table)
    }

    /// 等这一台**到过就绪那一格**——`Ready`，或之后被收掉的 `Stopping` / `Dead` 都算
    /// （**它答得动过**）。
    ///
    /// **有界**（照实记）：`wait` 是额度，`RETRY_MS` 一拍。它今天唯一的读者是装配那一趟
    /// "每条边成立没有"那一句（[`crate::system::Assembly::assemble`]）——**排对了就即刻返回**；
    /// 排错了（声明的边与实情不符）就当场报出"哪一台的哪条边"，不让客人自己去撞那圈有界重试。
    ///
    /// `Err(Fail::Unknown)` = 表里没这一行；`Err(Fail::NotReady)` = 到点还没到过。
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

    /// **收掉其余还活着的**：只对**还没下过刀**的那些（`Stopping` = 已下刀）逐位下令——故每一轮
    /// 都叫它是安全的，不会把 `doom` 刷成一场风暴。
    ///
    /// **下令即回**：等它们收讫是收场那一相自己的事（`core::done` 每拍读账），本手不逐行等。
    pub fn stop_rest(&mut self) {
        // 先把名字抄下来再动表（表是定长的、行数有上界——与 `sweep` 同一个形状）。
        let mut names = [const { String::new() }; Table::CAP];
        let mut n = 0usize;
        for row in self.table.living() {
            if !matches!(row.state, State::Starting | State::Ready) {
                continue;
            }
            // **本域那一枚不在这里收**：它的"域"就是本域，收它就是扑杀本域自己——它随本域
            // 退场时的"域亡＝成员清零"一起走（同 [`crate::system::control::supervise`] 里
            // `mark_dead` 那一格的对照）。
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
///
/// 只碰通道、不碰 `Control` 的任何一格，故是自由函数（`Control::connect` 那一层是白加的壳）。
pub fn connect(to: TaskId, ch: &'static str) -> Result<Endpoint, Error> {
    establish::endpoint(to, Mark::of(ch), Wait::POLL)
        .map_err(|_| Error::Step("connect failed"))
}

/// **名字 → 装配声明**：边上那四手要的那一份（`kind` / `setup` 都在它那里）。
///
/// 找不到 ⇒ `None`：这个名字不在这一景的装配声明里（`PROGRAMS` 是唯一声明处；帧里没有镜像，
/// 故"起哪一台"这件事只认本表）。
///
/// **只认"由编排域起"的那几台**（`relation.deps.is_some()`）：`root` / `system` 自己不在
/// 那张单里——运行期再造一枚"机器本身"不是本协议的意思。判据与 [`crate::system::assemble`]
/// 的过滤同一句。
fn program_of(name: &str) -> Option<&'static Program> {
    PROGRAMS
        .iter()
        .copied()
        .find(|p| p.relation.deps.is_some() && p.name() == name)
}
