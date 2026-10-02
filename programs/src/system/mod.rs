//! 运行时装配上下文（Assembly）＋ 它容纳的那几件。

use alloc::string::ToString;

use env::Wait;
use protocol::debug;

use crate::service::operator::bridge::Tree;
use crate::system::control::supervise::Watch;
use crate::system::control::{Control, E_TABLE, Error, READY_MS};
use crate::system::run::bootstrap::Boot;
use crate::unit::{Died, UnitFile};

use crate::system::common::face::mount;
use crate::system::run::schedule;

pub mod common;
pub mod control;
pub mod run;

/// **运行时装配上下文**：这台机器**已经装配到了什么**——四轴各一块，加存在信号的两头
/// 已成的关系）
pub struct Assembly {
    /// 生命轴：Service 的建 / 配 / 起 / 递单
    pub(crate) control: Control,
    /// 命名轴：持树者那一侧那条路
    pub(crate) tree: Tree,
    pub(crate) watch: Watch,
}

impl Assembly {
    /// 就位：建生命轴 ＋ 铸道立组（死亡道跟着这一景的装配表铸：**要存在信号的那几位一位一条**
    pub fn new(boot: Boot) -> Result<Assembly, ()> {
        Ok(Assembly {
            control: Control::new(boot.catalog, boot.machine, boot.accounts),
            tree: Tree::default(),
            watch: Watch::new()?,
        })
    }

    pub fn supervise(&mut self) -> bool {
        self.watch.run(&mut self.control, &mut self.tree)
    }

    /// **把 `control` 那一族挂上树**（`/svc/sys/control/{state,mint,start,stop}` 四面，一原语一面）
    /// 三步，次序即契约
    /// 2. **请持树者落**（Tree::plate）：把那一枚交过去，再把那条路推上提示之路
    /// 由树自己完成（它是那一格的权威）
    /// **失败只报一行读数、不拦整机**：挂不上是"这一面没有外面那条路"，不是"这台机器起不来"
    /// （与"某一台服务没接上板 / 树"同一口径）。三种失败各带自己的步名
    /// # `/svc/{族}` 自己不是一格
    /// 它是那条路上的**一段前缀**（第一条路的段列表走前缀时就地把它立成一块 `Pane`）——
    /// **没有它自己的入口、没有它的 Pie、也不是任何能力的别名**。故 `seek("/svc/sys/operator")`
    /// 之类答 Fail::NotATile：那一段是块窗格
    /// 到头了的是它底下那几格
    pub fn mount_control(&mut self) {
        let publication = self.control.hierarchy.borrow().entry;
        if let Some(entry) = publication {
            let road = protocol::common::path::Path::new("svc/sys/control/publish");
            let result = self.control.hierarchy.borrow_mut().internal(&mut self.tree, road, entry,
                protocol::service::operator::Permit::Public, runtime::env::unit::self_id(), self.control.roster.authority());
            if let Err(why) = result { debug::put(why); }
        }
        // State 公开；创建、启动、停止的发现边界是本实例明确安装的 Control 主体。
        // 不能用脱离 Identity authority 的裸 ROOT 编号表达安装权。
        for grant in protocol::system::control::Grant::ALL {
            let permit = match grant {
                protocol::system::control::Grant::State => protocol::service::operator::Permit::Public,
                _ => {
                    let Some(control) = self.control.roster.control() else {
                        debug::put("system: control identity not installed");
                        continue;
                    };
                    protocol::service::operator::Permit::Identity(
                        protocol::service::identity::Selector::Exact(control),
                    )
                }
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
            if let Err(why) = self.control.hierarchy.borrow_mut().internal(&mut self.tree, &road, entry, permit, runtime::env::unit::self_id(), self.control.roster.authority()) {
                debug::put(&alloc::format!("system: control face not mounted ({why})"));
                continue;
            }
            self.watch.attach_face(grant, entry);
            // 读数**从那条路自己打印**（`Path: Display`）——路径不再写第二遍。
            debug::put(&alloc::format!("system: control mounted at {road}"));
        }
    }

    /// **把七位操作面挂上树**（`/svc/sys/operator/{part,land,find,trim,list,seek,name}`）
    /// 那一族的常量给出**（`/svc/sys/operator`：**只是一段目录，不是任何能力的别名**：没有入口、
    /// 没有 Pie）。目录那几段由持树者**就地立出来**（`part` 幂等：缺的就地造，已在就是成了）
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
                self.control.hierarchy.borrow_mut().internal(&mut self.tree, &road, entry,
                    if matches!(grant, protocol::service::operator::Grant::Part | protocol::service::operator::Grant::Land | protocol::service::operator::Grant::Trim) { protocol::service::operator::Permit::Bound } else { protocol::service::operator::Permit::Public },
                    runtime::env::unit::self_id(), self.control.roster.authority())
            {
                debug::put(&alloc::format!("system: grant not mounted ({why})"));
                continue;
            }
            debug::put(&alloc::format!("system: grant mounted at {road}"));
        }
    }

    /// **起一条**——这一台自己的装配：**立账 → 造身子 → 装通道 → 走那三相**
    /// 前三件与"这一台是谁"无关（账上那一行 / 它那枚身子 / 它那几条通道），由
    /// Control 与 crate::system::control::enroll 做；三相各自动哪几手写在 schedule
    /// 那张表里，而**放行那一手夹在相与相之间**——它不是某一轴的手，是这一条自己的生命那一步
    /// **次序即契约**：先起的先就绪，后面的就能向它要东西；持树者必须先于客人（客人上树要它
    /// 在），Identity 必须先于其余（其后的身份都从它来）
    /// 失败一律折成这一台自己的 `died`（`UnitFile::demand.died`），读数靠那两行 debug
    /// （先印程序名、再印哪一步）
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
        let assembled = (|| {
            control::enroll::connect_all(program, &mut service).map_err(|e| fail(program, e))?;
            schedule::advance(self, schedule::BEFORE_LAUNCH, program, &mut service)?;
            self.control
                .launch(program, name, &mut service)
                .map_err(|e| fail(program, e))?;
            schedule::advance(self, schedule::AFTER_RELEASE, program, &mut service)?;
            schedule::advance(self, schedule::AFTER_READY, program, &mut service)
        })();
        if assembled.is_err() {
            self.control.discard(program.name(), service.0);
            let _ = self.control.progress(&mut self.tree);
        }
        assembled
    }
}

/// 报"哪一条、哪一步没成"，返**这一台自己的号**（crate::unit::Demand::died）
/// 只在失败路径上调：**成功不说话**（装配正常的机器不该刷屏），而失败时这两行决定还得读几遍
/// 代码——所以它报"程序名"与"步骤"两格
fn fail(program: &UnitFile, e: Error) -> Died {
    debug::put(program.name());
    debug::put(e.said());
    crate::system::control::E_PROGRAM
}
