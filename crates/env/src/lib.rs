#![no_std]
//! 环境调用 ABI（env）：**过线的那些东西**——U 态任务与 S 态域任务共用的调用封装
//! （slot 编码 + 载荷 codec + 线类型 + 发起骨架），加门闩权限（位掩码 + 两族视图）。
//!
//! **不含服务目录协议**（`Request`/`Reply`/`MSG_LEN`）——那是纯用户态协议，住在
//! `crates/protocol`（内核零引用；依赖方向 `kernel → env → runtime → protocol → programs`）。
//!
//! **"装机的账"也在本 crate**（照实记）：坐标 / 配对块 / 启动参数 / initrd 清单 / 供给词汇
//! （[`key`] / [`pair`] / [`args`] / [`manifest`] / [`supply`]）。它们与"过线的那些东西"的
//! 共同点只有一条：**宿主与 riscv 都编得过**。这几件原先被拆去 `crates/plan`（理由写的是
//! "env 是过线的、plan 是装机的"），而 **plan 作为程序装配中间层退场之后它们没有别处可放**
//! ——读同一批字节的两侧（宿主侧的 `crates/image`、riscv 侧的域）都编得过的只有本 crate。
//! 故它们回来了。**程序声明本身不在本 crate**：那是 `programs` 的 `Program` / `PROGRAMS`。
//!
//! 方案 3（typed payload）：各调用域枚举（`RoomCall` 等）是带类型载荷的 variant，
//! 字段类型为语义句柄（`PieToken`/`TaskId`/`VirtAddr`）或 `Permission`/裸量；
//! `[call]` 载荷由 `derive(Envcall)` 生成 codec（`slot/pack/from_wire/call`）。
//! 返回类型经 `#[ret(T)]` 标注，derive 生成域 `*Ret` 枚举。
//!
//! **面的两层**：`pub mod` 是**全部**，下面的 `pub use` 是**便利层**——收的是"调用点当词汇
//! 用"的那些名字（各调用域枚举、句柄、错误词汇、权限、名字、坐标、供给词汇…）。
//!
//! **本 crate 引 `alloc`**（照实记）：只为 [`manifest::pack`] 那段可增长的字节缓冲
//! （清单的写侧）。它随 plan 退场时一起回来；`entries` 那一侧（读侧）一个字节都不分配。

extern crate alloc;

pub mod args;
pub mod ecall;
pub mod exit;
pub mod fid;
pub mod key;
pub mod manifest;
pub mod pair;
pub mod permission;
pub mod supply;
pub mod wait;
pub mod wire;

pub use ecall::{FailCode, make_fail};
pub use exit::{EXIT_FAULT, EXIT_OK, EXIT_PANIC, Reason};
pub use fid::{
    ChronoCall, ChronoCallRet, ControlCall, ControlCallRet, ControlFail, ControlResult, DBCN_MAX,
    DebugCall, DebugCallRet, DebugFail, DebugResult, DispatchFail, EnvCall, HoleDir, MailCall,
    MailCallRet, MailFail, MailResult, MemoryCall, MemoryCallRet, MemoryFail, MemoryResult,
    NOTE_MAX, PieCall, PieCallRet, PieFail, PieResult, ProgramKind, RoomCall, RoomCallRet,
    RoomFail, RoomResult, ToleCall, ToleCallRet, ToleFail, ToleResult, UnitCall, UnitCallRet,
    UnitFail, UnitResult,
};
/// **每格一个精确签名的入口**（`#[derive(Envcall)]` 生成，一域一个模块）：
/// `env::memory::allocate(size)`、`env::pie::seal(token)`、`env::room::park(millis)`…
/// 载荷类型就是那一格的契约；标 `#[infallible]` 的格不返 `Result`。
pub use fid::{chrono, control, debug, mail, memory, pie, room, tole, unit};
/// **`Frame`**：定长帧的一处定义。实现在 `mold`（**过程宏**那一半），这里只转出来
/// ——故调用点写 `#[derive(env::Frame)]`（`protocol` 不依赖 `mold`，只能经这里取）。
pub use mold::Frame;
pub use key::{KEY_LEN, Key};
pub use pair::{PAIR_LEN, Pair};
pub use permission::{Access, Permission, Policy};
pub use supply::{At, Kind, Need, WANT_LEN, Want, class_block};
pub use wait::Wait;
pub use wire::{
    Decode, Eyes, FromPair, Mark, NAME_LEN, Name, NameError, PieToken, TaskId, TeamId, VirtAddr, Wire,
};
