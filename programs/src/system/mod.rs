//! system — **编排域的实现侧**：运行时装配上下文（[`Assembly`]）＋ 它容纳的那几件。
//! ```text
//!   Assembly
//!   ├── control   生命轴：Service 的建 / 配 / 起 / 递单（不认协议）
//!   ├── tree      命名轴：持树者那一侧那条路——号 / 提示之路 / 协调帧两格
//!   ├── roster    身份轴：名册那一面——门牌怎么认、谁补绑
//!   └── watch     存在信号·那一头：死亡道表 ＋ 等任一道响的那只组
//! ```

use alloc::string::ToString;

use env::Wait;
use protocol::debug;

use crate::service::operator::bridge::Tree;
use crate::service::principal::bridge::Roster;
use crate::system::bootstrap::Boot;
use crate::system::control::supervise::Watch;
use crate::system::control::{Control, E_TABLE, Error, READY_MS};
use crate::unit::{Died, UnitFile};

pub mod assemble;
pub mod bootstrap;
pub mod carrier;
pub mod control;
pub mod desk;
pub mod machine;
pub mod mount;
pub mod schedule;
pub mod source;

/// **运行时装配上下文**：这台机器**已经装配到了什么**——四轴各一块，加存在信号的两头。
/// 它不是"配置表"：配置在 `UnitFile` 上；这里只有**跑起来的东西**（已起的域与线程、已铸的孔、
/// 已成的关系）。
pub struct Assembly {
    /// 生命轴：Service 的建 / 配 / 起 / 递单。
    pub(crate) control: Control,
    /// 命名轴：持树者那一侧那条路。
    pub(crate) tree: Tree,
    /// 身份轴：名册那一面。
    pub(crate) roster: Roster,
    /// 存在信号·那一头：死亡道表与等任一道响的那只组。
    pub(crate) watch: Watch,
}

impl Assembly {
    /// 就位：建生命轴 ＋ 铸道立组（死亡道跟着这一景的装配表铸：**要存在信号的那几位一位一条**
    pub fn new(boot: Boot) -> Result<Assembly, ()> {
        Ok(Assembly {
            control: Control::new(boot.catalog, boot.machine, boot.accounts),
            tree: Tree::default(),
            roster: Roster::default(),
            watch: Watch::new()?,
        })
    }

    pub fn supervise(&mut self) -> bool {
        self.watch.run(&mut self.control)
    }

    /// **把 `control` 那一族挂上树**（`/svc/sys/control/{state,mint,start,stop}` 四面，一原语一面）：
    /// 本域逐面铸入口、**持树者逐面落那一格**、本域当场待客。
    /// 三步，次序即契约：
    /// 1. **铸入口**（[`crate::system::mount::entry`]）：本域主线程自己铸那一枚——它就是这一面的
    ///    服务端（入口的"开者"就是本域，客人 `Face::of` 据此知道往哪答话）；
    /// 2. **请持树者落**（[`Tree::plate`]）：把那一枚交过去，再把那条路推上提示之路；
    /// 3. **接上监督那一趟**（[`Watch::attach_face`]）：入口挂进同一只组，**本域当场开始待客**。
    /// **本域不进名册、也不开会话**：`land` 那道门是给**客人**的（本域不是客人），而"落"这一手
    /// 由树自己完成（它是那一格的权威）。
    /// **失败只报一行读数、不拦整机**：挂不上是"这一面没有外面那条路"，不是"这台机器起不来"
    /// （与"某一台服务没接上板 / 树"同一口径）。三种失败各带自己的步名。
    /// # `/svc/{族}` 自己不是一格
    /// 它是那条路上的**一段前缀**（第一条路的段列表走前缀时就地把它立成一块 `Pane`）——
    /// **没有它自己的入口、没有它的 Pie、也不是任何能力的别名**。故 `seek("/svc/sys/operator")`
    /// 之类答 [`Fail::NotATile`](protocol::service::operator::Fail::NotATile)：那一段是块窗格，
    /// 到头了的是它底下那几格。
    pub fn mount_control(&mut self) {
        // **四面各一枚入口、各一条路**（`/svc/sys/control/{state,mint,start,stop}`）——一原语一面。
        // **哪一面带规矩**：**问面公开**（`Rule::None`：谁都能问"这一条在哪个阶段"），
        // `mint` / `start` / `stop` 三面各带 [`Rule::Root`]——"**许给根**"（`Trunk(ROOT)`）。
        // **"哪一面带"那句话只此一处**：它住
        for grant in protocol::system::control::Grant::ALL {
            let rule = match grant {
                protocol::system::control::Grant::State => protocol::service::operator::Rule::None,
                _ => protocol::service::operator::Rule::Root,
            };
            let (entry, name) = match mount::entry(grant.mark(), grant.name()) {
                Ok(plate) => plate,
                Err(why) => {
                    debug::put(&alloc::format!("system: control face not mounted ({why})"));
                    continue;
                }
            };
            // 路：**本族那一族的常量**（`/svc/sys/control`）接上这一面的名——一处都不自己拼。
            let Some(road) = protocol::system::control::DIR.try_join(name.as_str()) else {
                debug::put("system: control face not mounted (name)");
                continue;
            };
            if let Err(why) = self.tree.plate(&road, Some(entry), rule) {
                debug::put(&alloc::format!("system: control face not mounted ({why})"));
                continue;
            }
            self.watch.attach_face(grant, entry);
            // 读数**从那条路自己打印**（`Path: Display`）——路径不再写第二遍。
            debug::put(&alloc::format!("system: control mounted at {road}"));
        }
    }

    /// **把七位操作面挂上树**（`/svc/sys/operator/{part,land,find,trim,list,seek,name}`）。
    /// 与 [`Assembly::mount_control`] 同一趟、同一只手（本域铸入口 → 持树者落格），**一路由本族
    /// 那一族的常量给出**（`/svc/sys/operator`：**只是一段目录，不是任何能力的别名**：没有入口、
    /// 没有 Pie）。目录那几段由持树者**就地立出来**（`part` 幂等：缺的就地造，已在就是成了）
    /// ——故**目录不单独占一帧**，它由第一位那条路的前缀走出来。
    /// **七格各自独立**：一位挂不上只少一位（各报一行读数、不拦整机），其余六位照挂。
    /// **本域不为任何一位开门待客**：这七格挂上去是给**别的域**用的——它们 `find` 回那一枚
    /// 入口，开在那一枚记号上的会话就是说给持树者的"我持这一柄权"。
    pub(crate) fn mount_grants(&mut self) {
        // 七位：每位一条路（`/svc/sys/operator/{name}`），前缀由持树者就地立出来。
        for grant in protocol::service::operator::Grant::ALL {
            let (entry, name) = match mount::entry(grant.mark(), grant.name()) {
                Ok(plate) => plate,
                Err(why) => {
                    debug::put(&alloc::format!("system: grant not mounted ({why})"));
                    continue;
                }
            };
            // 路：**本族那一族的常量**（`/svc/sys/operator`）接上这一面的名。
            let Some(road) = protocol::service::operator::DIR.try_join(name.as_str()) else {
                debug::put("system: grant not mounted (name)");
                continue;
            };
            if let Err(why) =
                self.tree
                    .plate(&road, Some(entry), protocol::service::operator::Rule::None)
            {
                debug::put(&alloc::format!("system: grant not mounted ({why})"));
                continue;
            }
            debug::put(&alloc::format!("system: grant mounted at {road}"));
        }
    }

    /// **起一条**——这一台自己的装配：**立账 → 造身子 → 装通道 → 走那三相**。
    /// 前三件与"这一台是谁"无关（账上那一行 / 它那枚身子 / 它那几条通道），由
    /// [`Control`] 与 [`crate::system::control::assemble`] 做；三相各自动哪几手写在 [`schedule`]
    /// 那张表里，而**放行那一手夹在相与相之间**——它不是某一轴的手，是这一条自己的生命那一步。
    /// **次序即契约**：先起的先就绪，后面的就能向它要东西；持树者必须先于客人（客人上树要它
    /// 在），名册必须先于其余（其后的身份都从它来）。
    /// 失败一律折成这一台自己的 `died`（`UnitFile::demand.died`），读数靠那两行 debug
    /// （先印程序名、再印哪一步）。
    pub fn assemble(&mut self, program: &UnitFile) -> Result<(), Died> {
        for dep in program.relation.after.unwrap_or(&[]) {
            if crate::unit::is_target(dep) {
                continue;
            }
            if self
                .control
                .await_ready(dep, Wait::AtMost(READY_MS))
                .is_err()
            {
                debug::put(&alloc::format!("system: dep not ready ({dep})"));
                return Err(fail(program, Error::Step("dep not ready")));
            }
        }

        // 登记：**"怎么算它起来了"由这一台的 `setup` 推出**。席满 / 名字非法 ⇒ 装配表那一格。
        self.control.enlist(program).map_err(|_| E_TABLE)?;

        let name = program.name().to_string();
        let mut service = self.control.spawn(program).map_err(|e| fail(program, e))?;

        // 通信：放行前把 `setup` 里那几条通道逐条装上（记号 = 通道名，放行后逐条认领）。
        // 一件一件来：`connect` 返的是**那条通道的持有者**（一次一手、一手一对孔）。
        control::assemble::connect_all(program, &mut service).map_err(|e| fail(program, e))?;

        // **三相**（表在 [`schedule`]）：放行那一手夹在相与相之间——它不是某一轴的手，是这一条
        // 自己的生命那一步（`launch` = 起步 ＋ 递整机物料，次序是硬的：物料要落到它交回的那条
        // 路上）。
        schedule::advance(self, schedule::BEFORE_LAUNCH, program, &mut service)?;
        self.control
            .launch(program, name, &mut service)
            .map_err(|e| fail(program, e))?;
        schedule::advance(self, schedule::AFTER_RELEASE, program, &mut service)?;
        schedule::advance(self, schedule::AFTER_READY, program, &mut service)?;
        Ok(())
    }
}

/// 报"哪一条、哪一步没成"，返**这一台自己的号**（[`crate::unit::Demand::died`]）。
/// 只在失败路径上调：**成功不说话**（装配正常的机器不该刷屏），而失败时这两行决定还得读几遍
/// 代码——所以它报"程序名"与"步骤"两格。
fn fail(program: &UnitFile, e: Error) -> Died {
    debug::put(program.name());
    debug::put(e.said());
    crate::system::control::E_PROGRAM
}
