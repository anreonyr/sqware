//! program — **静态的节点**：我是谁（`source`）＋ 实例化我需要什么（`setup`）。
//!
//! 它**不是**运行时实例：没有状态、没有生命周期、没有协议角色（board / operator /
//! holds_tree / eyes 那些是 scenario 里的**边**，不是节点的属性）。
//!
//! ```text
//!   Program（静态声明）── assemble() ──▶ Control ──▶ Service（运行时实例）
//! ```
//!
//! [`Program::assemble`] 只做三件事：**spawn → 逐条 `setup` → 返回 Service**。
//! `start` / `ready` / `supervise` / `stop` / `reap` 一律**不在这里**——那是 Control 的后续。

pub mod setup;
pub mod source;

pub use setup::Setup;
pub use source::{Role, Source, assembler};

use crate::system::control::{Control, Error, Service};

/// 一条服务的**静态声明**。
#[derive(Clone, Copy)]
pub struct Program {
    /// 清单里的程序名 —— 也是本域给它起的服务名（`Build` 的名字与清单名同源）。
    pub name: &'static str,
    /// 身子从哪来：镜像里的一台 / 住本域的一枚内件。
    pub source: Source,
    /// 实例化它要做的那几手（资源 / 通信）。
    pub setup: &'static [Setup],
}

impl Program {
    /// **把声明实例化**：spawn 一条服务，再把每一条 `setup` 落到它身上。
    ///
    /// **到 Service 为止**：不 start、不等就绪、不监督——那些是 Control/System 后续的事。
    ///
    /// 前置：这一行**已经登记过**（`Control::enlist`）——登记是"先立整张账"那一步，归 System。
    pub fn assemble(&self, control: &mut Control) -> Result<Service, Error> {
        let mut service = control.spawn(self.name, self.source)?;
        for setup in self.setup {
            setup.apply(control, &mut service)?;
        }
        Ok(service)
    }
}
