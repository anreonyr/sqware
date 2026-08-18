use crate::{eid, fid};

/// 用于绑定 EID 和 FID 类型的 trait
pub trait Extension {
    /// 该扩展的 EID
    const EID: usize;
    /// 该扩展的 FID 枚举类型
    type Fid: Into<usize> + Copy;
}

/// Base 扩展
pub struct BaseExt;
impl Extension for BaseExt {
    const EID: usize = eid::BASE;
    type Fid = fid::Base;
}

/// Timer 扩展
pub struct TimerExt;
impl Extension for TimerExt {
    const EID: usize = eid::TIME;
    type Fid = fid::Timer;
}

/// IPI 扩展
pub struct IpiExt;
impl Extension for IpiExt {
    const EID: usize = eid::IPI;
    type Fid = fid::Ipi;
}

/// RFENCE 扩展
pub struct RfenceExt;
impl Extension for RfenceExt {
    const EID: usize = eid::RFENCE;
    type Fid = fid::Rfence;
}

/// HSM 扩展
pub struct HsmExt;
impl Extension for HsmExt {
    const EID: usize = eid::HSM;
    type Fid = fid::Hsm;
}

/// System Reset 扩展
pub struct SystemResetExt;
impl Extension for SystemResetExt {
    const EID: usize = eid::SYSTEM_RESET;
    type Fid = fid::SystemReset;
}

/// PMU 扩展
pub struct PmuExt;
impl Extension for PmuExt {
    const EID: usize = eid::PMU;
    type Fid = fid::Pmu;
}

/// Debug Console 扩展
pub struct DbcnExt;
impl Extension for DbcnExt {
    const EID: usize = eid::DBCN;
    type Fid = fid::Dbcn;
}

/// Legacy Console 扩展（`sbi_console_putchar`，EID 0x01）
pub struct LegacyConsoleExt;
impl Extension for LegacyConsoleExt {
    const EID: usize = eid::LEGACY_CONSOLE_PUTCHAR;
    type Fid = fid::LegacyConsole;
}

/// System Suspend 扩展
pub struct SuspendExt;
impl Extension for SuspendExt {
    const EID: usize = eid::SUSP;
    type Fid = fid::Suspend;
}

/// CPPC 扩展
pub struct CppcExt;
impl Extension for CppcExt {
    const EID: usize = eid::CPPC;
    type Fid = fid::Cppc;
}

/// Nested Acceleration 扩展
pub struct NaClExt;
impl Extension for NaClExt {
    const EID: usize = eid::NACL;
    type Fid = fid::NaCl;
}
