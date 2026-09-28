//! system — **编排域的实现侧**：运行时装配上下文（[`Assembly`]）＋ 它容纳的那几件。
//!
//! ```text
//!   Assembly
//!   ├── control   生命轴：Service 的建 / 配 / 起 / 递单（不认协议）
//!   ├── tree      命名轴：持树者那一侧那条路——号 / 提示之路 / 协调帧两格
//!   ├── roster    身份轴：名册那一面——门牌怎么认、谁补绑
//!   ├── board     存在信号·这一头：板在装配者这一侧的那条路
//!   └── watch     存在信号·那一头：死亡道表 ＋ 等任一道响的那只组
//! ```
//!
//! **这些是字段，不是五枚壳**（照实记：这一刀把 `Life` / `Naming` / `Identity` / `Bloc` /
//! `Sensor` 五枚收掉了）。那五枚每一个都只做同一件事——把 `self.0.foo(...)` 再转发一遍，
//! 或者干脆把一个参数原样返回（`Bloc::report(who) -> who`）。**薄封装不是结构**：轴该是一个
//! **字段名**，而型该说"它是什么"（`Tree` = 持树者那一侧那条路），不该说"它在装配表里扮演
//! 什么"（`Naming` = 命名轴）。各轴各自的正文因此只在**它自己那一域**里
//! （`control/`、`operator/bridge.rs`、`principal/bridge.rs`、`board/`、`control/supervise.rs`）。
//!
//! **拆毒那一刀（照实记）**：从前这里平铺着 `control` / `board` / `tree` / `roster` / `watch`
//! 五个字段，而装配那一趟（原 `Program::assemble`）直接伸手进去乱叫——"生命"与"存在信号"
//! 两件事在同一段代码里交错，谁也说不清一次改动牵动谁。今天那一趟搬进 [`Assembly::assemble`]，
//! 且**每一块只经它自己那几手**说话（`enlist` / `spawn` / `connect_all` / `launch`、
//! `attach` / `adopt` / `plate` / `eye`、`bind` / `adopt`）：装配者手里没有一块是"想叫就叫"的
//! 裸**数据**——它叫的都是那一域自己的动词。
//!
//! **`Program` = 声明，`Assembly` = 运行时上下文**：一台程序怎么起（谁接哪条轴 / 要不要存在信号 /
//! 它是哪一双眼睛 / 装配期给不给身份）写在它自己那份 `program.rs` 里；而**装配动作**
//! 是 [`Assembly::assemble`]。
//!
//! **本文件里没有"按位分派"**：不再有一处 `if program.operator { … }` 的大 match 替所有程序
//! 解释它们的字段；属于哪一台的语义就在那一台的声明里，这一趟只按那几格走。
//!
//! - [`assemble`]：这一景起哪些台（**过滤 + 按 `order` 排**，就这一件事）
//! - [`bootstrap`]：启动资源获取（与引导域会话 / 机器自述 / 载荷区清单）
//! - [`source`]：程序来源那一格（取字节那一面：`Origin` → 一段 `&[u8]`）
//! - [`control`]：Service 的生命周期（内核那几手住 `control::service`，监督相住 `control::supervise`）
//! - [`board`] / [`operator`] / [`principal`] / [`coalition`]：四枚服务的实现
//! - [`machine`]：本域手里那台机器的自述（设备树）

use env::wire::Eyes;
use env::{Name, Wait};
use protocol::debug;

use runtime::env::unit as utask;

use protocol::system::control as ccall;

use crate::program::{Died, Program};
use crate::system::board::bridge::Bridge;
use crate::system::bootstrap::Boot;
use crate::system::control::supervise::Watch;
use crate::system::control::{Control, E_TABLE, Error, READY_MS};
use crate::system::operator::bridge::Tree;
use crate::system::principal::bridge::Roster;

pub mod assemble;
pub mod board;
pub mod bootstrap;
pub mod carrier;
pub mod coalition;
pub mod control;
// **照实记（这一册账是残枝那一刀从 protocol 搬来的）**：它原住
// `crates/protocol/src/system/desk.rs`。判据：那份账**只有编排域读**（两个消费者都在这里：
// `board/server.rs` 与 `operator/server.rs`），按 `protocol::driver` 那条"多个域都用 ≠ 该进
// protocol"的反面——**只有一个域用** ⇒ 回实现侧。
//
// **它为什么不像同来那一份那样归域**：同来的 `core.rs`（四条判定）与**服务表**那一半这一刀
// 各归了域（`control/core.rs` / `control/desk.rs`：它们只有生命轴一枚域读）；而**这一册待客账
// 是两枚域共用的一本**（板线程在编排域、持树者在 operator 域）⇒ 住它们共同的那一格。
pub mod desk;
pub mod machine;
pub mod operator;
pub mod principal;
pub mod source;

/// **运行时装配上下文**：这台机器**已经装配到了什么**——四轴各一块，加存在信号的两头。
///
/// 它不是"配置表"：配置在 `Program` 上；这里只有**跑起来的东西**（已起的域与线程、已铸的孔、
/// 已成的关系）。
pub struct Assembly {
    /// 生命轴：Service 的建 / 配 / 起 / 递单。
    control: Control,
    /// 命名轴：持树者那一侧那条路。
    tree: Tree,
    /// 身份轴：名册那一面。
    roster: Roster,
    /// 存在信号·这一头：板在装配者这一侧的那条路。
    board: Bridge,
    /// 存在信号·那一头：死亡道表与等任一道响的那只组。
    watch: Watch,
}

impl Assembly {
    /// 就位：建生命轴 ＋ 铸道立组（死亡道跟着这一景的装配表铸：**要存在信号的那几位一位一条**
    /// ——故道表在装配之前就位）；其余三轴是空的。
    ///
    /// 失败（那只组立不起来 / 备不下道表）由调用方折成 `system: no group`。
    pub fn new(boot: Boot, programs: &[&'static Program]) -> Result<Assembly, ()> {
        Ok(Assembly {
            control: Control::new(boot.catalog, boot.machine, boot.pier),
            tree: Tree::default(),
            roster: Roster::default(),
            board: Bridge::default(),
            watch: Watch::of(programs)?,
        })
    }

    /// 交棒给监督相之前，**先把 `control` 那一面挂上树**；道表与那只组都在 [`Watch`] 手里。
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
    /// **这一版把两格一起换**：铸入口的是编排域主线程（它此后就进 [`Watch::run`] 那一趟——
    /// **本域活多久它活多久**），而**落那一格由持树者在自己核里做**（本域只递：那一枚 ＋ 一条
    /// 路，见 [`Tree::plate`]）⇒ 那三处内核事实要的那一格（"铸入口那一枚必须长命"）满足了，
    /// 而**没有第三方上树**。
    ///
    /// **照实记（这条路经裁定：挂上树合设计）**：更早有一版判断是"`operator` 上树是坏事"——
    /// **那条判断不作准**：树就是"名字 → 资源"那本目录，谁要挂谁自己上来（今天就由树自己落）。
    /// 故 `control` 那一面**挂进树**（不是只靠装配期直授），取面方式与 `principal` / `coalition`
    /// 逐字同形；真客人是 `harness/src/probe_control.rs`。
    pub fn supervise(&mut self, last: Name) {
        self.mount_control();
        self.watch.run(&mut self.control, last);
    }

    /// **把 `control` 那一族挂上树**（`/sys/control/{state,mint,start,stop}` 四面，一原语一面）：
    /// 本域逐面铸入口、**持树者逐面落那一格**、本域当场待客。
    ///
    /// 三步，次序即契约：
    ///
    /// 1. **铸入口**（[`control::mount::entry`]）：本域主线程自己铸那一枚——它就是这一面的服务端
    ///    （入口的"开者"就是本域，客人 `Face::of` 据此知道往哪答话）；
    /// 2. **请持树者落**（[`Tree::plate`]）：把那一枚交过去，再把那条路推上提示之路；
    /// 3. **接上监督那一趟**（[`Watch::attach_face`]）：入口挂进同一只组，**本域当场开始待客**。
    ///
    /// **本域不进名册、也不开会话**：`land` 那道门是给**客人**的（本域不是客人），而"落"这一手
    /// 由树自己完成（它是那一格的权威）。
    ///
    /// **失败只报一行读数、不拦整机**：挂不上是"这一面没有外面那条路"，不是"这台机器起不来"
    /// （与"某一台服务没接上板 / 树"同一口径）。三种失败各带自己的步名。
    fn mount_control(&mut self) {
        let Ok(segment) = control::mount::segment() else {
            return debug!("system: control not mounted (control:name)");
        };
        let Ok(sys) = sys_dir() else {
            return debug!("system: control not mounted (sys:name)");
        };
        // **四面各一枚入口、各一条路**（`/sys/control/{state,mint,start,stop}`）——一原语一面。
        //
        // **哪一面带规矩**：**问面公开**（`Rule::None`：谁都能问"这一条在哪个阶段"）；
        // `mint` / `start` / `stop` 三面各带 [`Rule::Root`]——"**许给根**"（`Trunk(ROOT)`）：
        // 那正是 `control/mod.rs` 头注里那句"**要收，收的是那一格的 `Permit`**"——这一面上了树，
        // 而门禁原先只有"已绑身份"那一格 ⇒ 任何已绑身份的域都能 `mint` / `start` / `stop`
        // 装配表里任意一台；今天那三格收上了。
        for grant in protocol::system::control::Grant::ALL {
            let rule = match grant {
                protocol::system::control::Grant::State => protocol::system::operator::Rule::None,
                _ => protocol::system::operator::Rule::Root,
            };
            let (entry, name) = match control::mount::entry(grant) {
                Ok(plate) => plate,
                Err(why) => {
                    debug!("system: control face not mounted ({why})");
                    continue;
                }
            };
            let road = [sys, segment, name];
            if let Err(why) = self.tree.plate(&road, Some(entry), rule) {
                debug!("system: control face not mounted ({why})");
                continue;
            }
            self.watch.attach_face(grant, entry);
            debug!("system: control mounted at /sys/control/{}", grant.name());
        }
    }

    /// **把七位操作面挂上树**（`/sys/operator/{part,land,find,trim,list,seek,name}`）。
    ///
    /// 与 [`Assembly::mount_control`] 同一趟、同一只手（本域铸入口 → 持树者落格），但**一路三
    /// 段**：`/sys` → `/sys/operator`（**只是一段目录，不是任何能力的别名**：没有入口、没有
    /// Pie）→ `/sys/operator/{name}`。前两段由持树者**就地立出来**（`part` 幂等：缺的就地造，
    /// 已在就是成了）——故**目录不单独占一帧**，它由第一位那条路的前缀走出来。
    ///
    /// **七格各自独立**：一位挂不上只少一位（各报一行读数、不拦整机），其余六位照挂。
    ///
    /// **本域不为任何一位开门待客**：这七格挂上去是给**别的域**用的——它们 `find` 回那一枚
    /// 入口，开在那一枚记号上的会话就是说给持树者的"我持这一柄权"。
    fn mount_grants(&mut self) {
        // ① 树那一层 ＋ `operator` 那一段：两族共用前者（[`sys_dir`]），后者是本族自己的事实。
        let sys = match sys_dir() {
            Ok(name) => name,
            Err(why) => return debug!("system: grants not mounted ({why})"),
        };
        let segment = match operator::mount::segment() {
            Ok(name) => name,
            Err(why) => return debug!("system: grants not mounted ({why})"),
        };
        // ② 七位：每位一条路（`/sys/operator/{name}`），前缀由持树者就地立出来。
        for grant in protocol::system::operator::Grant::ALL {
            let (entry, name) = match operator::mount::entry(grant) {
                Ok(plate) => plate,
                Err(why) => {
                    debug!("system: grant not mounted ({why})");
                    continue;
                }
            };
            let road = [sys, segment, name];
            if let Err(why) = self.tree.plate(&road, Some(entry), protocol::system::operator::Rule::None) {
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
        // 登记：**"怎么算它起来了"由这一台的 `setup` 推出**。席满 / 名字非法 ⇒ 装配表那一格。
        self.control.enlist(program).map_err(|_| E_TABLE)?;

        let name = Name::new(program.name()).map_err(|_| fail(program, Error::Manifest))?;
        let mut service = self.control.spawn(program).map_err(|e| fail(program, e))?;

        // 通信：放行前把 `setup` 里那几条通道逐条装上（记号 = 通道名，放行后逐条认领）。
        // 一件一件来：`connect` 返的是**那条通道的持有者**（一次一手、一手一对孔）。
        control::assemble::connect_all(program, &mut service).map_err(|e| fail(program, e))?;

        // 身份：**放行之前**就做完——故服务一起来 `resolve(self)` 就答得出。（名册本身与树不
        // 走这里：它们起来时名册还没在；那两条由下面的身份轴 `adopt` 在它放行之后补绑。）
        self.roster
            .bind(service.0, program.relation.bind)
            .map_err(|why| fail(program, Error::Step(why)))?;

        // 放行 + 等就绪 + 递门闩单（次序是硬的：配给要落到它交回的那条路上）。
        self.control
            .launch(program, name, &mut service)
            .map_err(|e| fail(program, e))?;

        // 存在信号：**板在装配者这一侧那条路**——把这位客人交出来的那一枚转授过去（板据此
        // 看得见它的死）。**在通道之后**：那条路由客人在起来之后自己装（它是问的那一侧），而它
        // 要先收到配给才轮得到那一问。
        if program.relation.presence {
            let lane = self.watch.lane_of(name.as_str());
            self.board
                .attach(utask::self_id(), service.0, name, Wait::AtMost(READY_MS), lane)
                .map_err(|why| fail(program, Error::Step(why)))?;
        }

        // 命名轴：**按需**把这条服务接到持树者那棵树上。**在存在信号之后**：两者各一条路、
        // 互不影响。持树者必须先于这位客人起：提示之路还没认下就没得接。
        if program.relation.operator {
            self.tree
                .attach(service.0, Wait::AtMost(READY_MS))
                .map_err(|why| fail(program, Error::Step(why)))?;
        }

        // 它刚把提示之路交给**生我者**（= 本域）⇒ 当场认下来，此后客人上树才有路可走。
        if program.relation.holds_tree {
            self.tree
                .adopt(service.0, Wait::AtMost(READY_MS))
                .map_err(|why| fail(program, Error::Step(why)))?;
            // **持树者一就位就把七位挂上**（不是等整表起完）：
            // 那七格只是"树 + 本域递东西"两件事的函数，与后面起哪几台无关；而**等整表起完**会把
            // 它挤到最后——那正是停机扳机（`supervise` 的 `last`）响的前一刻 ⇒ 任何"要读那七格"
            // 的客人只剩几毫秒窗口（实测：那一档里连既有的 `probe-control` 都会被扑杀）。
            self.mount_grants();
        }

        // **哪一双眼睛**：声明上那一格说了算（不是拿名字认的——`p.name == "principal"`
        // 那种写法，改个名字就静默失灵）。名册那一位要认下面 + 补绑自己与树；盟册只报号。
        //
        // **盟册那一步没有第五个字段**：那个号装配者本来就握着（`service.0`），"报到命名轴去"
        // 只是把同一枚号记进协调帧那一格——故这里就是那一句。
        match program.relation.eyes {
            Some(Eyes::Roster) => {
                let who = self
                    .roster
                    .adopt(service.0, self.tree.host())
                    .map_err(|why| fail(program, Error::Step(why)))?;
                self.tree.eye(Eyes::Roster, who);
            }
            Some(Eyes::League) => self.tree.eye(Eyes::League, service.0),
            None => {}
        }

        Ok(())
    }
}

/// **树那一层那一格**（`/sys`）：`control` 与 `operator` 两族的路都从它起。
///
/// 正文是 [`ccall::frame::DIR`]（"各族挂在 `/sys` 底下"那一条的原文）——持树者那一侧从前还有一份
/// 私有副本（`CONTROL_DIR`），两份字符串互不相识、各写各的；这一刀起只有这一处给。
fn sys_dir() -> Result<Name, &'static str> {
    Name::new(ccall::frame::DIR).map_err(|_| "system:sys")
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
