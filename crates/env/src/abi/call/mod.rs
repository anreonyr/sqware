//! Typed environment calls use `(class << 32) | slot` and six argument words.
//! Hole, Pole, Nole and Tole own resource operations; Pie owns capability operations.
//! Classes 3 and 9 are unassigned. Unknown classes and slots return `Decode::BadSlot`.

pub mod chrono;
pub mod control;
pub mod debug;
mod dispatch_fail;
pub mod hole;
mod hole_condition;
mod hole_fail;
mod hole_limits;
pub mod memory;
pub mod nole;
mod nole_fail;
mod oversize;
pub mod pie;
mod pie_info;
pub mod pole;
mod pole_bit;
mod pole_condition;
mod pole_fail;
mod pull_outcome;
mod release_mode;
pub mod room;
pub mod tole;
mod tole_await_reply;
mod tole_event_fail;
mod tole_event_source;
mod tole_fail;
pub mod unit;

pub use self::chrono::{ChronoCall, ChronoCallRet};
pub use self::control::{ControlCall, ControlCallRet, ControlFail, ControlResult};
pub use self::debug::{DBCN_MAX, DebugCall, DebugCallRet, DebugFail, DebugResult};
pub use self::dispatch_fail::DispatchFail;
pub use self::hole::{HoleCall, HoleCallRet, HoleCondition, HoleFail, HoleLimits, HoleResult};
pub use self::memory::{
    AcquireSource, Allocation, MemoryCall, MemoryCallRet, MemoryFail, MemoryResult, Placement,
    PlacementAt, PlacementError, ReferenceRange, ViewControl, ViewSelection,
};
pub use self::nole::{NoleCall, NoleCallRet, NoleFail, NoleResult};
pub use self::oversize::Oversize;
pub use self::pie::{PieCall, PieCallRet, PieFail, PieResult};
pub use self::pie_info::PieInfo;
pub use self::pole::{Bit, PoleCall, PoleCallRet, PoleCondition, PoleFail, PoleResult};
pub use self::pull_outcome::PullOutcome;
pub use self::release_mode::ReleaseMode;
pub use self::room::{NOTE_MAX, RoomCall, RoomCallRet, RoomFail, RoomResult};
pub use self::tole::{
    ToleAwaitReply, ToleCall, ToleCallRet, ToleEventFail, ToleEventSource, ToleFail, ToleResult,
};
pub use self::unit::{UnitCall, UnitCallRet, UnitFail, UnitResult};

/// Decoded environment call.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EnvCall {
    Room(self::room::RoomCall),
    Unit(self::unit::UnitCall),
    Memory(self::memory::MemoryCall),
    Chrono(self::chrono::ChronoCall),
    Hole(self::hole::HoleCall),
    Nole(self::nole::NoleCall),
    Pole(self::pole::PoleCall),
    Tole(self::tole::ToleCall),
    Control(self::control::ControlCall),
    Pie(self::pie::PieCall),
    /// 调试面（见 [`self::debug::DebugCall`]）。
    Debug(self::debug::DebugCall),
}

impl EnvCall {
    /// 由调用号 + 寄存器组解码回带载荷的聚合枚举。
    pub fn from_wire(slot: usize, regs: &[usize; 6]) -> Result<Self, crate::wire::Decode> {
        let class = slot >> 32;
        match class {
            0 => Ok(EnvCall::Room(self::room::RoomCall::from_wire(slot, regs)?)),
            1 => Ok(EnvCall::Unit(self::unit::UnitCall::from_wire(slot, regs)?)),
            2 => Ok(EnvCall::Memory(self::memory::MemoryCall::from_wire(
                slot, regs,
            )?)),
            4 => Ok(EnvCall::Chrono(self::chrono::ChronoCall::from_wire(
                slot, regs,
            )?)),
            5 => Ok(EnvCall::Hole(self::hole::HoleCall::from_wire(slot, regs)?)),
            6 => Ok(EnvCall::Control(self::control::ControlCall::from_wire(
                slot, regs,
            )?)),
            7 => Ok(EnvCall::Pie(self::pie::PieCall::from_wire(slot, regs)?)),
            8 => Ok(EnvCall::Debug(self::debug::DebugCall::from_wire(
                slot, regs,
            )?)),
            10 => Ok(EnvCall::Pole(self::pole::PoleCall::from_wire(slot, regs)?)),
            11 => Ok(EnvCall::Nole(self::nole::NoleCall::from_wire(slot, regs)?)),
            12 => Ok(EnvCall::Tole(self::tole::ToleCall::from_wire(slot, regs)?)),
            _ => Err(crate::wire::Decode::BadSlot),
        }
    }
}
