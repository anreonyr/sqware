//! call — **环境调用号那一张表**：契约与聚合（本模块）＋ **一域一份**（`room`/`unit`/`memory`/
//! `chrono`/`mail`/`pie`/`control`/`debug`：每份 = 该域的 `*Call` ＋ `*Fail` ＋ `Result` 别名）。
//!
//! 方案 3（typed payload）：每个原语是一个**带类型载荷的 variant**，字段类型是
//! envcall 语义句柄（`PieToken`/`TaskId`/`VirtAddr`）或 `Permission`/裸量。调用号
//! （a7）不再 `#[repr(usize)]`+手写 `as usize`，而由 `[derive(Envcall)]` 生成的
//! codec 现算：`(class << 32) | slot`。Pie、Mail 通过 `#[slot(N)]` 明确指定操作号；
//! 其他调用域仍按声明顺序编号。
//!
//! 返回类型（R3）：每个 variant 标 `#[ret(T)]`，derive 生成域 `*Ret` 枚举与
//! 每格一个入口（负值读回**该域的词汇**，非负蒸馏成那一格的载荷）。入口绑定 envcall 汇编，
//! `slot/pack/unpack` 只依赖 `Wire`——sbi 未来可复用同一 derive。
//!
//! 分类按**操作的归属轴**一一对应（class=高 32 位）：Room=0, Unit=1, Memory=2,
//! Chrono=4, Mail=5, Control=6, **Pie=7**, Debug=8；旧 Tole class=9 拒绝。命名与调度词族
//! （conductor）、内核 `runtime::chrono` 域及用户侧 execution 同词。
//!
//! **class 3 空着不补**：设备不是内核的事——域持门闩、自己读写寄存器，控制台是服务。
//! class 号保持原来的归属。
//! 空号即"这条路上没有内核的入口"，这比复用更准确。
//!
//! **5 与 7 的分界是两条正交的轴**（不是按资源种类分，也不是按新旧分）：
//! - **class 5 `Mail` = 数据轴**：消息穿孔。`Push`/`Pull`/`Wait`——传的是**内容**。
//! - **class 7 `Pie` = 权柄轴**：权柄的生死与流动。`Unseal`/`Seal`/`Open`/`Shut`
//!   /`Accord`/`Narrow`/`Revoke`/`Collect`/`Inspect`/`Release`——传的是**许可**。
//!
//! 两轴正交的判据在代码里：数据轴的臂从**不**调用 `gate` 的权柄函数
//! （`accord`/`narrow`/`revoke`/`release`/`vestor`/`snap`），权柄轴的臂从**不**搬运
//! 载荷。
//! `Pie` 复用原 `ServiceCall` 的空出的 7 号（后者是入口策略而非原语：目录入口门闩
//! 改由父任务 `Accord` 下发）。
//!
//! **未知调用号的运行时契约（ABI 的一部分，不是实现细节）**：`a7` 由调用方
//! 完全控制，故它是**输入**而非可信标识。未声明的 class / index（今天 class 3、9
//! 与 ≥ 10 的号段是空的、以及越界索引）一律 decoded 为 `Decode::BadSlot`，
//! 内核侧按**被拒绝**处理：写回负码（`Fail::Denied`）并**续跑调用方**——
//! 与其它用户引起的异常同走故障隔离，绝不 panic（否则用户态一发 `ebreak`
//! 即可停摆整机）。想主动终止有正规原语 `RoomCall::Reap { reason }`——它退的是
//! **调用方那一枚线程**（同域其它线程照旧；域亡与否由成员清零决定），内核照旧活着。
//! 本模块是这条契约的**单一真相**：`slot` 的生成与解码都在此处。
//!
//! 根除的两处 L3' 漏洞：**本 crate 这一处**是 `Permission` 的 unpack 走
//! `from_bits(...).ok_or(...)` 校验（见 [`Wire`](crate::wire::Wire)）；另一处是内核的
//! `PteFlags`（`kernel/src/runtime/switcher/envcall/mod.rs` 的 `Mprotect` 那一格，
//! 它不在本 crate 的 `Wire` 面里）。两处都非法位 → `Err`，不再 `from_bits_truncate`
//! 静默截断。

use mold::Fail;

pub mod chrono;
pub mod control;
pub mod debug;
pub mod mail;
pub mod memory;
pub mod pie;
pub mod pie_types;
pub mod room;
pub mod unit;

pub use self::chrono::{ChronoCall, ChronoCallRet};
pub use self::control::{ControlCall, ControlCallRet, ControlFail, ControlResult};
pub use self::debug::{DBCN_MAX, DebugCall, DebugCallRet, DebugFail, DebugResult};
pub use self::mail::{AwaitReply, Bit, MailCondition, Oversize, PullOutcome, Source};
pub use self::mail::{MailCall, MailCallRet, MailFail, MailResult};
pub use self::memory::{MemoryCall, MemoryCallRet, MemoryFail, MemoryResult};
pub use self::pie::{PieCall, PieCallRet, PieFail, PieResult};
pub use self::pie_types::{HoleLimits, PieInfo, ReleaseMode, UnsealArgs};
pub use self::room::{NOTE_MAX, RoomCall, RoomCallRet, RoomFail, RoomResult};
pub use self::unit::{UnitCall, UnitCallRet, UnitFail, UnitResult};

/// **无域那一层**（dispatch）：`EnvCall::from_wire` 失败——调用号读不懂。
///
/// 它不是任何域的失败：发生在我们还不知道这是哪一域的时候。与各域首码同为 `-1`，
/// 区分靠"回答发生在选定域之前"；良构调用到不了这一格（slot 由生成的代码给出，
/// 内核表与域同镜像 ⇒ 真到 = 不变量破裂）。
#[derive(Fail)]
pub enum DispatchFail {
    /// 未声明的 class / 越界索引。
    Unknown = -1,
}

/// 环境调用号聚合（内核侧解码总入口）。
///
/// `from_wire(slot, regs)` 按 class（高 32 位）分派到各域的 `from_wire`，得到
/// `self::pie::PieCall::Seal { .. }` 等带载荷 variant，供 `dispatch` match。用户侧不再构造
/// 本枚举——直接 `self::pie::PieCall::X.call()` 发起（R3+B）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EnvCall {
    Room(self::room::RoomCall),
    Unit(self::unit::UnitCall),
    Memory(self::memory::MemoryCall),
    Chrono(self::chrono::ChronoCall),
    Mail(self::mail::MailCall),
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
            5 => Ok(EnvCall::Mail(self::mail::MailCall::from_wire(slot, regs)?)),
            6 => Ok(EnvCall::Control(self::control::ControlCall::from_wire(
                slot, regs,
            )?)),
            7 => Ok(EnvCall::Pie(self::pie::PieCall::from_wire(slot, regs)?)),
            8 => Ok(EnvCall::Debug(self::debug::DebugCall::from_wire(
                slot, regs,
            )?)),
            _ => Err(crate::wire::Decode::BadSlot),
        }
    }
}
