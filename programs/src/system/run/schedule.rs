//! 一台服务从"立了账、造了身子"到"在树上答得动"，分几相走、每相谁动手。
//! ```text
//!   立账 → 造身子 → 装通道        ← 不是手：这三件对整张单逐条做（`control/{assemble,mod}.rs`）

use crate::service::operator::bridge as operator;
use crate::service::identity::bridge as identity;
use crate::system::Assembly;
use crate::system::control::enroll as control;
use crate::system::control::{Error, Service};
use crate::unit::{Died, UnitFile};

/// **一手**：某一轴在某一相里对某一台做的一件事
/// **它返的是一句话，不是一个号**：装配失败的号是按**这一台**分的（`UnitFile::demand.died`）
/// 由 advance 折出来；手只报"死在装配哪一步"（与 service::step 同款）
pub type Act = fn(&mut Assembly, &UnitFile, &mut Service) -> Result<(), &'static str>;

/// **一相**：这一相里那几只手，**次序即契约**
pub type Phase = &'static [Act];

pub const BEFORE_LAUNCH: Phase = &[identity::bind, prepare_runtime, crate::harness::probe::identity::supply];

pub const AFTER_RELEASE: Phase = &[control::await_ready];

/// Local ready 后确认 Operator，再安装 Identity 的可信查询束与完整挂载。
/// AFTER_READY 全部成功，才放行依赖本系统服务就绪的后续单位。
pub const AFTER_READY: Phase = &[operator::hold, identity::adopt];

/// **走一相**：逐手；哪一手不成，折成**这一台自己的号**（读数 = 程序名 ＋ 那一手自己的步名）
pub fn advance(
    assembly: &mut Assembly,
    phase: Phase,
    program: &UnitFile,
    service: &mut Service,
) -> Result<(), Died> {
    for &act in phase {
        act(assembly, program, service)
            .map_err(|why| crate::system::fail(program, Error::Step(why)))?;
    }
    Ok(())
}

fn prepare_runtime(assembly: &mut Assembly, _program: &UnitFile, _service: &mut Service) -> Result<(), &'static str> {
    assembly.control.progress(&mut assembly.tree)
}
