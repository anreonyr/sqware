//! 一台服务从"立了账、造了身子"到"在树上答得动"，分几相走、每相谁动手。
//! ```text
//!   立账 → 造身子 → 装通道        ← 不是手：这三件对整张单逐条做（`control/{assemble,mod}.rs`）

use crate::service::operator::bridge as operator;
use crate::service::principal::bridge as principal;
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

pub const BEFORE_LAUNCH: Phase = &[principal::bind];

pub const AFTER_RELEASE: Phase = &[operator::attach_client, control::await_ready];

/// **它答得动之后**：认下"答案从哪来"那一类事实——谁持树（那一枚提示之路）、谁是名册
/// （它交上来的那一枚定面门牌）。**两件都由运行期的那一枚孔认**，不读声明
pub const AFTER_READY: Phase = &[operator::hold, principal::adopt_roster];

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
