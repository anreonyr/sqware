#![no_std]
//! 环境调用 ABI（env）：U 态任务与 S 态域任务共用的调用封装（slot 编码 + 载荷
//! codec + 线类型 + 发起骨架），独立共享 crate。
//!
//! **不含服务目录协议**（`Request`/`Reply`/`MSG_LEN`）——那是纯用户态协议，住在
//! `task/src/core/dispatch.rs`（内核零引用；见 docs §10.21）。
//!
//! 方案 3（typed payload）：各调用域枚举（`RoomCall` 等）是带类型载荷的 variant，
//! 字段类型为语义句柄（`PieToken`/`TaskId`/`VirtAddr`）或 `Permission`/裸量；
//! `[call]` 载荷由 `derive(Envcall)` 生成 codec（`slot/pack/unpack/call`）。
//! 返回类型经 `#[ret(T)]` 标注，derive 生成域 `*Ret` 枚举。

pub mod ecall;
pub mod fid;
pub mod permission;
pub mod wire;

pub use ecall::{EnvError, EnvResult, make_err};
pub use fid::{
    ChronoCall, ChronoCallRet, ControlCall, ControlCallRet, EnvCall, HoleDir, IOCall, IOCallRet,
    MailCall, MailCallRet, MemoryCall, MemoryCallRet, PieCall, PieCallRet, ProgramKind, RoomCall,
    RoomCallRet, UnitCall, UnitCallRet,
};
pub use permission::Permission;
pub use wire::{
    Decode, FromPair, NAME_LEN, Name, NameError, PieToken, TaskId, TeamId, VirtAddr, Wire,
};
