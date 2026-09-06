#![no_std]
//! U-mode → S-mode 环境调用封装（ubi），独立共享 crate。
//!
//! 方案 3（typed payload）：各调用域枚举（`RoomCall` 等）是带类型载荷的 variant，
//! 字段类型为语义句柄（`PieToken`/`TaskId`/`VirtAddr`）或 `Permission`/裸量；
//! `[call]` 载荷由 `derive(Envcall)` 生成 codec（`slot/pack/unpack/call`）。
//! 返回类型经 `#[ret(T)]` 标注，derive 生成域 `*Ret` 枚举。

pub mod fid;
pub mod permission;
pub mod spawnee;
pub mod ucall;
pub mod wire;

pub use fid::{
    ChronoCall, ChronoCallRet, ControlCall, ControlCallRet, EnvCall, IOCall, IOCallRet,
    MailCall, MailCallRet, MemoryCall, MemoryCallRet, RoomCall, RoomCallRet, UnitCall,
    UnitCallRet,
};
pub use permission::Permission;
pub use spawnee::Spawnee;
pub use ucall::{EnvError, EnvResult};
pub use wire::{Decode, FromPair, PieToken, TaskId, TeamId, VirtAddr, Wire};
