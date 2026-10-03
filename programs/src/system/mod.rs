//! 运行时装配上下文（Assembly）＋ 它容纳的那几件。

use alloc::string::ToString;
use alloc::{boxed::Box, sync::Arc};
use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use env::Wait;
use protocol::debug;

use crate::system::operator::bridge::Tree;
use crate::system::control::supervise::Watch;
use crate::system::control::{Control, E_TABLE, Error, READY_MS};
use crate::system::run::bootstrap::Boot;
use crate::unit::{Died, UnitFile};

use crate::system::common::face::mount;

pub mod common;
pub mod control;
pub mod identity;
pub mod operator;
pub mod run;
pub mod publication;
pub mod runtime;

#[repr(u8)]
#[derive(Clone, Copy)]
pub enum Phase { Starting, Running, Stopping }

pub struct Status {
    pub(crate) control: env::TaskId,
    pub(crate) operator: AtomicUsize,
    pub(crate) identity: AtomicUsize,
    pub(crate) phase: AtomicU8,
}

/// Control task 持有外部 team 生命周期及跨任务装配事务。
pub struct Assembly {
    /// 生命轴：Service 的建 / 配 / 起 / 递单
    pub(crate) control: Control,
    /// 命名轴：持树者那一侧那条路
    pub(crate) tree: Tree,
    pub(crate) watch: Watch,
    pub(crate) publication: publication::Publication,
    pub(crate) runtime: runtime::Runtime,
    pub(crate) names: identity::names::Names,
}

impl Assembly {
    /// 建立同 team 的 Operator、Identity task 并完成三者的引导装配。
    pub fn new(boot: Boot) -> Result<Assembly, ()> {
        let status = Arc::new(Status {
            control: ::runtime::env::unit::self_id(),
            operator: AtomicUsize::new(0),
            identity: AtomicUsize::new(0),
            phase: AtomicU8::new(Phase::Starting as u8),
        });
        let publication = publication::Publication::new();
        let mut assembly = Assembly {
            control: Control::new(boot.catalog, boot.machine, boot.accounts, status.clone(), publication.entry),
            tree: Tree::default(),
            watch: Watch::new()?,
            publication,
            runtime: runtime::Runtime::new(),
            names: identity::names::Names::new(),
        };
        // Publish both task identities before either task is released.
        for (slot, operator) in [(&status.operator, true), (&status.identity, false)] {
            let state = status.clone();
            let body: Box<dyn FnOnce(usize) + Send> = Box::new(move |_| {
                let success = if operator {
                    operator::serve::serve(state.clone()).is_ok()
                } else {
                    identity::serve::serve(state.clone()).is_ok()
                };
                if !success || state.phase.load(Ordering::Acquire) != Phase::Stopping as u8 {
                    debug::put("system: internal task failed; terminating team");
                    let _ = ::runtime::env::room::doom(::runtime::env::unit::self_id());
                }
            });
            let ptr = Box::into_raw(Box::new(body));
            let task = match ::runtime::env::unit::spawn(env::TeamId::new(0),
                ::runtime::core::task::join::trampoline as *const () as usize, &[ptr as usize], 0)
            {
                Ok(task) => task,
                Err(_) => {
                    // SAFETY: Spawn failed; no task can consume this closure.
                    unsafe { drop(Box::from_raw(ptr)); }
                    let _ = ::runtime::env::room::doom(status.control);
                    return Err(());
                }
            };
            slot.store(task.get(), Ordering::Release);
        }
        let operator = env::TaskId::new(status.operator.load(Ordering::Acquire));
        let identity = env::TaskId::new(status.identity.load(Ordering::Acquire));
        if ::runtime::env::unit::hatch(operator).is_err() || ::runtime::env::unit::hatch(identity).is_err() {
            let _ = ::runtime::env::room::doom(status.control);
            return Err(());
        }
        let installed = (|| {
            assembly.tree.adopt(operator, Wait::AtMost(control::BOOT_MS))?;
            identity::bridge::install(&mut assembly.control.roster, &mut assembly.tree, &mut assembly.publication, &mut assembly.names, identity)?;
            assembly.mount_grants()?;
            assembly.mount_control()?;
            Ok::<(), &'static str>(())
        })();
        if let Err(why) = installed {
            debug::put(why);
            let _ = ::runtime::env::room::doom(status.control);
            return Err(());
        }
        status.phase.store(Phase::Running as u8, Ordering::Release);
        debug::put("system: Control, Operator and Identity ready in one team");
        Ok(assembly)
    }

    pub fn supervise(&mut self) -> Result<(), control::supervise::Fail> {
        let result = self.watch.run(&mut self.control, &mut self.publication, &mut self.runtime, &mut self.names, &mut self.tree);
        if let Err(why) = result {
            let _ = ::runtime::env::room::doom(self.control.status.control);
            return Err(why);
        }
        let status = &self.control.status;
        status.phase.store(Phase::Stopping as u8, Ordering::Release);
        let until = ::runtime::env::chrono::clock() + control::BOOT_MS as u64 * 1_000_000;
        for task in [status.operator.load(Ordering::Acquire), status.identity.load(Ordering::Acquire)] {
            let task = env::TaskId::new(task);
            while !::runtime::env::unit::join(task, Wait::POLL).unwrap_or(true) {
                if ::runtime::env::chrono::clock() >= until {
                    let _ = ::runtime::env::room::doom(status.control);
                    return Err(control::supervise::Fail::Shutdown);
                }
                let _ = ::runtime::env::room::sleep(core::time::Duration::from_millis(1));
            }
        }
        debug::put("system: internal tasks stopped");
        Ok(())
    }

    pub fn mount_control(&mut self) -> Result<(), &'static str> {
        let entry = self.publication.entry.ok_or("publication entry missing")?;
        let me = ::runtime::env::unit::self_id();
        let authority = self.control.roster.authority();
        self.publication.internal(&mut self.tree,
            protocol::common::path::Path::new("svc/sys/control/publish"), entry,
            protocol::system::operator::Permit::Public, me, authority)?;
        let principal = self.control.roster.control().ok_or("Control identity missing")?;
        for grant in protocol::system::control::Grant::ALL {
            let permit = if grant == protocol::system::control::Grant::State {
                protocol::system::operator::Permit::Public
            } else {
                protocol::system::operator::Permit::Identity(protocol::system::identity::Selector::Exact(principal))
            };
            let (entry, _) = mount::entry(grant.mark(), grant.name())?;
            let road = protocol::system::control::DIR.try_join(grant.name()).ok_or("Control path")?;
            self.publication.internal(&mut self.tree, &road, entry, permit, me, authority)?;
            self.watch.attach_face(grant, entry);
            debug::put(&alloc::format!("system: control mounted at {road}"));
        }
        Ok(())
    }

    fn mount_grants(&mut self) -> Result<(), &'static str> {
        for grant in protocol::system::operator::Grant::ALL {
            let (entry, _) = mount::entry(grant.mark(), grant.name())?;
            let road = protocol::system::operator::DIR.try_join(grant.name()).ok_or("Operator path")?;
            let permit = if matches!(grant, protocol::system::operator::Grant::Part
                | protocol::system::operator::Grant::Land | protocol::system::operator::Grant::Trim)
            { protocol::system::operator::Permit::Bound } else { protocol::system::operator::Permit::Public };
            self.publication.internal(&mut self.tree, &road, entry, permit,
                ::runtime::env::unit::self_id(), self.control.roster.authority())?;
        }
        Ok(())
    }

    /// 按声明建立外部 team，安装身份、资源与发布入口，再放行并等候就绪。
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
            self.control.authorize_static(service.0).map_err(|why| fail(program, Error::Step(why)))?;
            if let Some(prepare) = program.prepare {
                prepare(self.control.roster.authority(), program, service.0)
                    .map_err(|why| fail(program, Error::Step(why)))?;
            }
            self.publication.poll(&self.control.table, &self.control.roster, &self.control.machine, &self.control.static_tasks, &mut self.runtime, &mut self.names, &mut self.tree).map_err(|why| fail(program, Error::Step(why)))?;
            self.control.launch(program, name.clone(), &mut service).map_err(|e| fail(program, e))?;
            self.control.ready(name, &mut service, program.supply(),
                |control| self.publication.poll(&control.table, &control.roster, &control.machine, &control.static_tasks, &mut self.runtime, &mut self.names, &mut self.tree)).map_err(|e| fail(program, e))

        })();
        if assembled.is_err() {
            self.control.discard(program.name(), service.0);
            let _ = self.publication.poll(&self.control.table, &self.control.roster, &self.control.machine, &self.control.static_tasks, &mut self.runtime, &mut self.names, &mut self.tree);
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
