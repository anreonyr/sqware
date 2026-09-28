//! system — **编排域的实现侧**：运行时装配上下文（[`Assembly`]）＋ 它容纳的那几件。
//!
//! ```text
//!   Assembly
//!   ├── life      生命轴：Service 的创建 / 配置 / 启动 / 递单（不认协议）
//!   ├── naming    命名轴：持树者（operator）那一侧——号 / 提示之路 / 协调帧两格
//!   ├── identity  身份轴：名册（principal）那一面——门牌怎么认、谁补绑
//!   ├── bloc      横向轴：盟册（coalition）——装配期只报"第二双眼睛是谁"
//!   └── sensor    存在信号：板那一枚传感器 ＋ 它监视的死亡道表与那只组
//! ```
//!
//! **四轴 + 一枚传感器**，每块只管自己那一句话：前四块各是协议里的一条轴，各自有自己那几手
//! （`enlist` / `spawn` / `connect` / `launch`、`attach` / `adopt` / `eye`、`bind` / `adopt`、
//! `report`）；`sensor` **不是第五条轴**——它是那枚传感器（板，`Bridge`）与它监视的那几条道
//! （`Watch`），见 [`Sensor`] 的头注。
//!
//! **拆毒那一刀（照实记）**：从前这里平铺着 `control` / `board` / `tree` / `roster` / `watch`
//! 五个字段，而装配那一趟（原 `Program::assemble`）直接伸手进去乱叫——"生命"与"存在信号"
//! 两件事在同一段代码里交错，谁也说不清一次改动牵动谁。今天那一趟搬进 [`Assembly::assemble`]，
//! 且**只经各块自己的手**说话：装配者手里没有一块是"想叫就叫"的裸字段。
//!
//! **`Program` = 声明，`Assembly` = 运行时上下文**：一台程序怎么起（谁接哪条轴 / 要不要存在信号 /
//! 它是哪一双眼睛 / 装配期给不给身份）写在它自己那份 `program.rs` 里；而**装配动作**是
//! [`Assembly::assemble`]——它按那一台的声明把那一台装进各块。[`Program::assemble`] 只转发。
//!
//! **本文件里没有"按位分派"**：不再有一处 `if program.operator { … }` 的大 match 替所有程序
//! 解释它们的字段；属于哪一台的语义就在那一台的 `assemble` 一趟里。
//!
//! - [`assemble`]：这一景起哪些台（**过滤 + 按 `order` 排**，就这一件事）
//! - [`bootstrap`]：启动资源获取（与引导域会话 / 机器自述 / 载荷区清单）
//! - [`source`]：程序来源那一格（取字节那一面：`Origin` → 一段 `&[u8]`）
//! - [`control`]：Service 的生命周期（内核那几手住 `control::service`，监督相住 `control::supervise`）
//! - [`board`] / [`operator`] / [`principal`] / [`coalition`]：四枚服务的实现
//! - [`machine`]：本域手里那台机器的自述（设备树）

use env::TaskId;
use env::wire::Eyes;
use env::{Name, PieToken, Wait};
use protocol::debug;

use runtime::env::unit as utask;

use crate::program::{Died, Program, Setup};
use crate::system::board::bridge::Bridge;
use crate::system::bootstrap::Boot;
use crate::system::control::supervise::Watch;
use crate::system::control::{Control, E_TABLE, Error, READY_MS, Service, connect};
use crate::system::operator::bridge::Tree;
use crate::system::principal::bridge::Roster;

pub mod assemble;
pub mod board;
pub mod bootstrap;
pub mod coalition;
pub mod control;
// **照实记（`core` / `desk` 是残枝那一刀从 protocol 搬来的）**：它们原住
// `crates/protocol/src/system/{desk,core}.rs`。判据：那份账与那几条判定**只有编排域读**
// （两个消费者都在本目录：`board/server.rs` 与 `operator/server.rs`），按
// `protocol::driver` 那条"多个域都用 ≠ 该进 protocol"的反面——**只有一个域用** ⇒ 回实现侧。
pub mod core;
pub mod desk;
pub mod machine;
pub mod operator;
pub mod principal;
pub mod source;

/// **运行时装配上下文**：这台机器**已经装配到了什么**——四轴各一块，加那一枚存在信号传感器。
///
/// 它不是"配置表"：配置在 `Program` 上；这里只有**跑起来的东西**（已起的域与线程、已铸的孔、
/// 已成的关系）。
pub struct Assembly {
    /// 生命轴：Service 的建 / 配 / 起 / 递单。
    life: Life,
    /// 命名轴：持树者那一侧。
    naming: Naming,
    /// 身份轴：名册那一面。
    identity: Identity,
    /// 横向轴：盟册（装配期只有那一句话）。
    bloc: Bloc,
    /// 存在信号：板 ＋ 死亡道表与那只组。
    sensor: Sensor,
}

impl Assembly {
    /// 就位：铸道 + 立组（在 [`Sensor::of`]，包着 `Watch::of`）→ 建生命轴；其余三轴是空的。
    ///
    /// `programs` = 这一景真有的那几台（[`assemble::programs`]）。死亡道跟着它铸：**要存在信号的
    /// 那几位一位一条**——故道表在装配之前就位。
    ///
    /// 失败（那只组立不起来 / 备不下道表）由调用方折成 `system: no group`。
    pub fn new(boot: Boot, programs: &[&'static Program]) -> Result<Assembly, ()> {
        Ok(Assembly {
            life: Life(Control::new(boot.catalog, boot.machine, boot.pier)),
            naming: Naming(Tree::default()),
            identity: Identity(Roster::default()),
            bloc: Bloc,
            sensor: Sensor::of(programs)?,
        })
    }

    /// 交棒给监督相之前，**先把 `control` 那一面挂上树**；道表与那只组都在 [`Sensor`] 手里。
    ///
    /// **照实记（这一刀：`control` 那一面挂回来了，两格一起换）**：task-4 那条挂载路
    /// （`control::edge::mount`：起一枚**一次性**边沿线程去落门牌）**挂不出真正的门牌**，被撤过
    /// 一次。它坏在"铸入口的是谁"，证据链在内核三处：
    ///
    /// - `kernel/src/work/unit/gate/accord.rs`：**新 pie 的 `sire = Some(src.token())`**，
    ///   派生边只写在这里；
    /// - `kernel/src/boot.rs` 的 `EXIT_HOOKS`：**每个** reaped 任务都跑 `gate::doom`；
    /// - `kernel/src/work/unit/gate/cull.rs::doom`：先 `seal_owned`，再对本任务表里**每一枚
    ///   token** 调 `cull`，而 `cull` 沿 `snap::heirs`（`p.sire() == Some(token)`）**跨所有
    ///   任务**摘掉全部后代。
    ///
    /// 于是"铸入口那一枚线程"一收尾，**持树者表里那枚入口副本被连带摘掉**；plate 那一格还留着
    /// 那个号 ⇒ 查得到、门闩却拿不回来 ⇒ 客人永远拿不到可用入口。
    ///
    /// **这一版把两格一起换**：铸入口的是编排域主线程（它此后就进 [`Sensor::run`] 那一趟——
    /// **本域活多久它活多久**），而**落那一格由持树者在自己核里做**（本域只递：那一枚 ＋ 两段
    /// 名字，见 [`Naming::land_plate`]）⇒ 那三处内核事实要的那一格（"铸入口那一枚必须长命"）
    /// 满足了，而**没有第三方上树**。
    ///
    /// **照实记（这条路经裁定：挂上树合设计）**：更早有一版判断是"`operator` 上树是坏事"——
    /// **那条判断不作准**：树就是"名字 → 资源"那本目录，谁要挂谁自己上来（今天就由树自己落）。
    /// 故 `control` 那一面**挂进树**（不是只靠装配期直授），取面方式与 `principal` / `coalition`
    /// 逐字同形；真客人是 `harness/src/probe_control.rs`。
    pub fn supervise(&mut self, last: Name) {
        self.mount_control();
        self.sensor.run(&mut self.life, last);
    }

    /// **把 `control` 那一面挂上树**（`/sys/control`）：本域铸那一枚入口、**持树者落那一格**、
    /// 本域当场待客。
    ///
    /// 三步，次序即契约：
    ///
    /// 1. **铸入口**（[`control::mount::entry`]）：本域主线程自己铸那一枚——它就是这一面的服务端
    ///    （入口的"开者"就是本域，客人 `Face::of` 据此知道往哪答话）；
    /// 2. **请持树者落**（[`Naming::land_plate`]）：把那一枚交过去，再把两段名字推上提示之路；
    /// 3. **接上监督那一趟**（[`Sensor::attach_face`]）：入口挂进同一只组，**本域当场开始待客**。
    ///
    /// **本域不进名册、也不开会话**：`land` 那道门是给**客人**的（本域不是客人），而"落"这一手
    /// 由树自己完成（它是那一格的权威）。
    ///
    /// **失败只报一行读数、不拦整机**：挂不上是"这一面没有外面那条路"，不是"这台机器起不来"
    /// （与"某一台服务没接上板 / 树"同一口径）。三种失败各带自己的步名。
    fn mount_control(&mut self) {
        let (entry, dir, name) = match control::mount::entry() {
            Ok(plate) => plate,
            Err(why) => return debug!("system: control not mounted ({why})"),
        };
        if let Err(why) = self.naming.land_plate(entry, dir, name) {
            return debug!("system: control not mounted ({why})");
        }
        self.sensor.attach_face(entry);
        debug!("system: control mounted at /sys/control");
    }

    /// **把七位操作面挂上树**（`/sys/operator/{part,land,find,trim,list,seek,name}`）。
    ///
    /// 与 [`Assembly::mount_control`] 同一趟、同一只手（本域铸入口 → 持树者落格），但**两层**：
    /// 先把 `/sys/operator` 那一段立成一块 `Pane`（**它只是目录，不是任何能力的别名**），七位再
    /// 落在它底下（[`Naming::land_deep`]）。
    ///
    /// **七格各自独立**：一位挂不上只少一位（各报一行读数、不拦整机），其余六位照挂。目录那一段
    /// 挂不上 ⇒ 七位都挂不上（各报它自己的那一行）；那一步失败就地收工，不逐位重试。
    ///
    /// **本域不为任何一位开门待客**：这七格挂上去是给**别的域**用的——它们 `find` 回那一枚
    /// 入口，开在那一枚记号上的会话就是说给持树者的"我持这一柄权"。服务端判面那一句见
    /// `programs/src/system/operator/server.rs::answer`（第一道闸）。
    fn mount_grants(&mut self) {
        // ① 目录那一段：`/sys/operator`（自己也是一格——有名字、有门闩）。
        let (segment, _dir, segment_name) = match operator::mount::pane() {
            Ok(plate) => plate,
            Err(why) => return debug!("system: grants not mounted ({why})"),
        };
        //    **目录那一格也是一层"深处"**：它落在 `/sys` 底下，故走同一手（`land_deep`）——
        //    只是这位面的名字就是这段目录自己，故两段名字**同一个**（`land_deep` 要的那个
        //    等式，见那边）。
        if let Err(why) = self.naming.land_deep(segment, segment, segment_name, segment_name) {
            return debug!("system: grants not mounted ({why})");
        }
        // ② 七位：`operator` 底下那七段。
        for grant in protocol::system::operator::Grant::ALL {
            let (entry, mid, name) = match operator::mount::entry(grant) {
                Ok(plate) => plate,
                Err(why) => {
                    debug!("system: grant not mounted ({why})");
                    continue;
                }
            };
            if let Err(why) = self.naming.land_deep(entry, segment, mid, name) {
                debug!("system: grant not mounted ({why})");
                continue;
            }
            debug!("system: grant mounted at /sys/operator/{}", grant.name());
        }
    }

    /// **起一条**——这一台自己的装配，按它自己的声明走：
    ///
    /// 立账 → 建域产线程 → 装通道（放行前）→ 绑身份（放行前）→ 放行等就绪 → 递配给 → 存在信号 →
    /// 命名轴 → 认下那两双眼睛。
    ///
    /// **次序即契约**：先起的先就绪，后面的就能向它要东西；持树者必须先于客人（客人上树要它
    /// 在），名册必须先于其余（其后的身份都从它来）。
    ///
    /// 失败一律折成这一台自己的 `died`（`Program::demand.died`），读数靠那两行 debug
    /// （先印程序名、再印哪一步）。
    pub fn assemble(&mut self, program: &Program) -> Result<(), Died> {
        // 登记：**"怎么算它起来了"由这一台的 `setup` 推出**。席满 / 名字非法 ⇒ 装配表那一格
        // （与旧 `enlist` 那一趟同号，与具体哪一台无关）。
        self.life.enlist(program).map_err(|_| E_TABLE)?;

        let name = Name::new(program.name()).map_err(|_| fail(program, Error::Manifest))?;
        let mut service = self.life.spawn(program).map_err(|e| fail(program, e))?;

        // 通信：放行前把 `setup` 里那几条通道逐条装上（记号 = 通道名，放行后逐条认领）。
        // 一件一件来：`connect` 返的是**那条通道的持有者**（一次一手、一手一对孔）。
        self.life
            .connect(program, &mut service)
            .map_err(|e| fail(program, e))?;

        // 身份：**放行之前**就做完——故服务一起来 `resolve(self)` 就答得出。（名册本身与树不
        // 走这里：它们起来时名册还没在；那两条由下面的身份轴 `adopt` 在它放行之后补绑。）
        self.identity
            .bind(service.0, program.relation.bind)
            .map_err(|why| fail(program, Error::Step(why)))?;

        // 放行 + 等就绪（有通道的那一条顺带逐条认领）；再递门闩单。
        self.life
            .launch(program, name, &mut service)
            .map_err(|e| fail(program, e))?;

        // 存在信号：本域是那枚传感器的宿主 ⇒ 把客人交出来的那一枚转授过去。**在通道之后**：
        // 那条路由客人在起来之后自己装（它是问的那一侧），而它要先收到配给才轮得到那一问。
        if program.relation.presence {
            self.sensor
                .attach(service.0, name)
                .map_err(|why| fail(program, Error::Step(why)))?;
        }

        // 命名轴：**按需**把这条服务接到持树者那棵树上。**在存在信号之后**：两者各一条路、
        // 互不影响。持树者必须先于这位客人起：提示之路还没认下就没得接。
        if program.relation.operator {
            self.naming
                .attach(service.0)
                .map_err(|why| fail(program, Error::Step(why)))?;
        }

        // 它刚把提示之路交给**生我者**（= 本域）⇒ 当场认下来，此后客人上树才有路可走。
        if program.relation.holds_tree {
            self.naming
                .adopt(service.0)
                .map_err(|why| fail(program, Error::Step(why)))?;
            // **持树者一就位就把七位挂上**（不是等整表起完）：
            // 那七格只是"树 + 本域递东西"两件事的函数，与后面起哪几台无关；而**等整表起完**会把
            // 它挤到最后——那正是停机扳机（`supervise` 的 `last`）响的前一刻 ⇒ 任何"要读那七格"
            // 的客人只剩几毫秒窗口（实测：那一档里连既有的 `probe-control` 都会被扑杀）。
            self.mount_grants();
        }

        // **哪一双眼睛**：声明上那一格说了算（不是拿名字认的——`p.name == "principal"`
        // 那种写法，改个名字就静默失灵）。名册那一位要认下面 + 补绑自己与树；盟册只报号。
        if let Some(eyes) = program.relation.eyes {
            let who = match eyes {
                Eyes::Roster => self
                    .identity
                    .adopt(service.0, self.naming.host())
                    .map_err(|why| fail(program, Error::Step(why)))?,
                Eyes::League => self.bloc.report(service.0),
            };
            self.naming.eye(eyes, who);
        }

        Ok(())
    }
}

/// **生命轴**：Service 的建 / 配 / 起 / 递单。
///
/// 它是 [`Control`] 在装配上下文里的那一格；这一块的手**就是**生命周期的四相，别处不再有第二套
/// 叫法（协议语义一概不在这块——那是其余三轴与传感器的边）。
struct Life(Control);

impl Life {
    /// 登记一行（`Control::enlist`）：**"怎么算起来"由这一行的 `setup` 推出**。
    fn enlist(&mut self, program: &Program) -> Result<(), Error> {
        self.0.enlist(program.name(), program.demand.setup)
    }

    /// 按名字去清单里挑镜像 → 建域 → 产线程（`Control::spawn`）。前置：这一行已登记过。
    ///
    /// **来源那一格随这一手一起过去**：字节从哪本来由这一台自己的声明说（`demand.origin`）。
    fn spawn(&mut self, program: &Program) -> Result<Service, Error> {
        self.0.spawn(program.name(), program.demand.origin)
    }

    /// 放行前把 `setup` 里那几条通道逐条装上：`connect` 一次一件、一手一对孔。
    fn connect(&mut self, program: &Program, service: &mut Service) -> Result<(), Error> {
        for s in program.demand.setup {
            if let Setup::Channel(ch) = s {
                service
                    .1
                    .try_reserve(1)
                    .map_err(|_| Error::Step("no room for channels"))?;
                let channel = connect(service.0, ch)?;
                service.1.push(channel);
            }
        }
        Ok(())
    }

    /// 放行 + 等就绪 + 递门闩单（`Control::start` 与 `Control::wire` 合成一相）。
    ///
    /// **次序是硬的**：配给要落到它交回的那条路上，故递单只能在放行之后。
    fn launch(&mut self, program: &Program, name: Name, service: &mut Service) -> Result<(), Error> {
        self.0.start(name, service, program.demand.setup)?;
        self.0.wire(name, service, program.demand.setup)
    }
}

/// **命名轴**：持树者（`operator`）那一侧——号 / 提示之路 / 协调帧两格。
///
/// 它答"这一位叫什么、挂在哪"，故"哪一双眼睛是谁"的协调帧也归它记（[`Naming::eye`]）。
struct Naming(Tree);

impl Naming {
    /// 持树者那一枚的号（`None` = 还没起）。
    fn host(&self) -> Option<TaskId> {
        self.0.host()
    }

    /// 把这位客人接上树；持树者还没起就没得接。
    fn attach(&mut self, task: TaskId) -> Result<(), &'static str> {
        self.0.attach(task, Wait::AtMost(READY_MS))
    }

    /// 它就是持树者本身：认下它那条提示之路，此后客人上树才有路可走。
    fn adopt(&mut self, host: TaskId) -> Result<(), &'static str> {
        self.0.adopt(host, Wait::AtMost(READY_MS))
    }

    /// **要持树者替本域落一格**（`control` 那一面那一格）：那一枚 ＋ 两段名字。
    ///
    /// 正文在 [`Tree::land_plate`](crate::system::operator::bridge::Tree::land_plate)：
    /// 本域**不上树**——落由持树者在自己核里做。
    fn land_plate(&mut self, entry: PieToken, dir: Name, name: Name) -> Result<(), &'static str> {
        self.0.land_plate(entry, dir, name)
    }

    /// **落两层**（`/sys/operator/{op}` 那一族）：`dir` 那一段先立成 `Pane`，`name` 落在它底下。
    ///
    /// 正文在 [`Tree::land_deep`](crate::system::operator::bridge::Tree::land_deep)。
    fn land_deep(
        &mut self,
        entry: PieToken,
        segment: PieToken,
        dir: Name,
        name: Name,
    ) -> Result<(), &'static str> {
        self.0.land_deep(entry, segment, dir, name)
    }

    /// 它是哪一双眼睛：那一格记进给持树者的协调帧（重复推是幂等的）。
    fn eye(&mut self, eyes: Eyes, who: TaskId) {
        self.0.eye(eyes, who);
    }
}

/// **身份轴**：名册（`principal`）那一面。
///
/// 它答"这一位此刻代表谁"，故装配期每一条服务的身份都从这条路上来；名册自己放行之后才认下面，
/// 并补绑它自己与树（它们起来时名册还没在）。
struct Identity(Roster);

impl Identity {
    /// **放行前**给这一条服务派一条号、绑到它那一枚线程上。
    fn bind(&self, task: TaskId, on: bool) -> Result<(), &'static str> {
        self.0.bind(task, on)
    }

    /// **名册自己放行之后**：认下它交给生我者的那一面，补绑它自己与树，返它的号。
    fn adopt(&mut self, task: TaskId, tree: Option<TaskId>) -> Result<TaskId, &'static str> {
        self.0.adopt(task, tree)
    }
}

/// **横向轴**：盟册（`coalition`）在装配期的位置。
///
/// **照实记（这一块为什么没有状态）**：盟册是**成员之间**的服务——装配期本域既不持它的手柄，
/// 也不替它说话；它与装配那一趟的唯一来路是"**第二双眼睛是谁**"（`Eyes::League`），而那一格
/// 记在命名轴的协调帧里（[`Naming::eye`]）。按本仓那条"**没有读者的格不留在面上**"的规矩
/// （见 [`crate::system::core`] 里 `Watch` / `probe_watch` 退场那一笔），这一块**不摆状态**：
/// 它只留那一手——把横向那一位域报到命名轴去。
struct Bloc;

impl Bloc {
    /// 横向那一双眼睛**只报号**（门牌与门禁由盟册那一族自己走）。
    fn report(&self, who: TaskId) -> TaskId {
        who
    }
}

/// **存在信号**（一枚传感器）：板那一枚死信号传感器 ＋ 它监视的那几条道。
///
/// **照实记（板为什么与道表同块）**：板那一侧（[`Bridge`]：那位客人交回的那一枚经它转授）与
/// 监督那一相（[`Watch`]：死亡道表 ＋ 等任一道响的那只组）答的是**同一件事的两头**——客人接上
/// 板（`Bridge::attach`），它没了就在它那条道上出一格（`Watch::run` 记账）。从前它们是
/// `Assembly` 上两个平铺的字段（`board` / `watch`），于是"存在信号"这件事被切成两处；收进这一块
/// 之后它只有一处可记，而"要不要存在信号"仍由每一台自己的 `relation.presence` 说。
struct Sensor {
    /// 板在装配者这一侧的手柄（那条提示之路）。
    board: Bridge,
    /// 死亡道表与那只组。
    watch: Watch,
}

impl Sensor {
    /// **铸道 + 立组**：要存在信号的那几位一位一条（记号 `LANE_PREFIX` ＋ 名字）。
    ///
    /// 组是**独占**的（`shared = false`）：监督那一趟用它等任一道响（零轮询）。
    /// 失败由调用方折成 `system: no group`。
    fn of(programs: &[&'static Program]) -> Result<Sensor, ()> {
        Ok(Sensor {
            board: Bridge::default(),
            watch: Watch::of(programs)?,
        })
    }

    /// 把这一位接上板：板线程按需起（只一枚），它那条死亡道跟着转授过去。
    ///
    /// 返 `Err(哪一步没成)`——对调用方是同一件事（这一条服务没接上板），但"死在哪一步"正是装配
    /// 诊断要的那一格。
    fn attach(&mut self, task: TaskId, name: Name) -> Result<(), &'static str> {
        let lane = self.watch.lane_of(name.as_str());
        self.board
            .attach(utask::self_id(), task, name, Wait::AtMost(READY_MS), lane)
    }

    /// 交棒给监督相（道表与那只组都在 [`Watch`] 手里；生命轴那一本表是它要改的账）。
    fn run(&mut self, life: &mut Life, last: Name) {
        self.watch.run(&mut life.0, last);
    }

    /// **把 `control` 那一面接上监督那一趟**：入口挂进同一只组。
    ///
    /// 两源（道表 ＋ 面）那形状住 [`Watch`]（`attach_face` 与它那一格 `face`）；本块只是把
    /// "这一枚是 control 的入口"这句话转过去——**存在信号的传感器与它监视的那几条道同块**，
    /// 而那一条待客的路与道共用的正是同一只组（同一个等待）。
    fn attach_face(&mut self, face: PieToken) {
        self.watch.attach_face(face);
    }
}

impl Program {
    /// **起一条**：**只转发**——装配那一趟是 [`Assembly::assemble`]（`Program` 是声明，
    /// 它不该自带装配那一套知识；这一手留着，是为了让调用点读起来仍是"这一台自己起"）。
    pub fn assemble(&self, assembly: &mut Assembly) -> Result<(), Died> {
        assembly.assemble(self)
    }
}

/// 报"哪一条、哪一步没成"，返**这一台自己的号**（[`crate::program::Demand::died`]）。
///
/// 只在失败路径上调：**成功不说话**（装配正常的机器不该刷屏），而失败时这两行决定还得读几遍
/// 代码——所以它报"程序名"与"步骤"两格。
fn fail(program: &Program, e: Error) -> Died {
    debug!("{}", program.name());
    debug!("{}", e.said());
    program.demand.died
}
