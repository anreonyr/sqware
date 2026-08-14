#![allow(unused)]

use crate::sbi::{extension::*, scall::ScallBuilder};

pub mod eid;
pub mod extension;
pub mod fid;
pub mod scall;

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
