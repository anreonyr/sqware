#![no_std]
//! S-mode → M-mode 的 SBI 调用封装（sbi），独立共享 crate。

pub mod eid;
pub mod extension;
pub mod fid;
pub mod scall;

use extension::*;
use scall::*;

pub type BaseCall = ScallBuilder<BaseExt>;
pub type TimerCall = ScallBuilder<TimerExt>;
pub type IpiCall = ScallBuilder<IpiExt>;
pub type RfenceCall = ScallBuilder<RfenceExt>;
pub type HsmCall = ScallBuilder<HsmExt>;
pub type SystemResetCall = ScallBuilder<SystemResetExt>;
pub type PmuCall = ScallBuilder<PmuExt>;
pub type DbcnCall = ScallBuilder<DbcnExt>;
pub type LegacyConsoleCall = ScallBuilder<LegacyConsoleExt>;
pub type SuspendCall = ScallBuilder<SuspendExt>;
pub type CppcCall = ScallBuilder<CppcExt>;
pub type NaClCall = ScallBuilder<NaClExt>;
