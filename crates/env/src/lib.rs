#![no_std]
//! 环境调用 ABI（env）：**过线的那些东西**——U 态任务与 S 态域任务共用的调用封装
//! （slot 编码 + 载荷 codec + 线类型 + 发起骨架），加门闩权限（位掩码 + 两族视图）。
//!
//! **不含服务目录协议**（`Request`/`Reply`/`MSG_LEN`）——那是纯用户态协议，住在
//! `crates/protocol`（内核零引用；依赖方向 `kernel → env → runtime → protocol → programs`）。
//!
//! 方案 3（typed payload）：各调用域枚举（`RoomCall` 等）是带类型载荷的 variant，
//! 字段类型为语义句柄（`PieToken`/`TaskId`/`VirtAddr`）或 `Permission`/裸量；
//! `[call]` 载荷由 `derive(Envcall)` 生成 codec（`slot/pack/from_wire/call`）。
//! 返回类型经 `#[ret(T)]` 标注，derive 生成域 `*Ret` 枚举。
//!
//! **面的两层**：`pub mod` 是**全部**，下面的 `pub use` 是**便利层**——收的是"调用点当词汇
//! 用"的那些名字（各调用域枚举、句柄、错误词汇、权限、名字、坐标、供给词汇…）。
//!
//! **串面就是 std 的那两枚**：借 `&str`、拥有 `String`——本仓**一个字都不新写**，
//! 也没有第三个可转的名字（C 那一族是 C ABI 的形状，不是名字的）。线上那一格名字就是
//! `String` 自己的 `Span` impl：**长度那一字节 ＋ 那几字节**；定宽、终止 NUL、零填充都不在了。
//! **名字不报上界**（那一格 `MAX = None`）：它多长由**族**说——带它的帧写 `#[frame(len = …)]`。

extern crate alloc;

pub mod abi;
pub mod ecall;
pub mod ledger;
pub mod wire;

pub use ecall::{FailCode, make_fail};
pub use abi::exit::{EXIT_FAULT, EXIT_OK, EXIT_PANIC, Reason};
pub use abi::call::{
    ChronoCall, ChronoCallRet, ControlCall, ControlCallRet, ControlFail, ControlResult, DBCN_MAX,
    DebugCall, DebugCallRet, DebugFail, DebugResult, DispatchFail, EnvCall, HoleDir, MailCall,
    MailCallRet, MailFail, MailResult, MemoryCall, MemoryCallRet, MemoryFail, MemoryResult,
    NOTE_MAX, PieCall, PieCallRet, PieFail, PieResult, RoomCall, RoomCallRet,
    RoomFail, RoomResult, ToleCall, ToleCallRet, ToleFail, ToleResult, UnitCall, UnitCallRet,
    UnitFail, UnitResult,
};
/// **每格一个精确签名的入口**（`#[derive(Envcall)]` 生成，一域一个模块）：
/// `env::memory::allocate(size)`、`env::pie::seal(token)`、`env::room::park(millis)`…
/// 载荷类型就是那一格的契约；标 `#[infallible]` 的格不返 `Result`。
pub use abi::call::{chrono::chrono, control::control, debug::debug, mail::mail,
    memory::memory, pie::pie, room::room, tole::tole, unit::unit};
pub use ledger::key::{KEY_LEN, Key};
/// **`Frame`**：定长帧的一处定义。实现在 `mold`（**过程宏**那一半），这里只转出来
/// ——故调用点写 `#[derive(env::Frame)]`（`protocol` 不依赖 `mold`，只能经这里取）。
pub use mold::Frame;
pub use ledger::pair::{PAIR_LEN, Pair};
pub use abi::permission::{Access, Permission, Policy};
pub use wire::pie_kind::PieKind;
pub use wire::program_kind::ProgramKind;
pub use abi::wait::Wait;
pub use wire::{Decode, FromPair, Mark, PieToken, TaskId, TeamId, VirtAddr, Wire};
