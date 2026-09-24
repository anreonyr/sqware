#![no_std]
//! 环境调用 ABI（env）：U 态任务与 S 态域任务共用的调用封装（slot 编码 + 载荷
//! codec + 线类型 + 发起骨架），独立共享 crate。
//!
//! **不含服务目录协议**（`Request`/`Reply`/`MSG_LEN`）——那是纯用户态协议，住在
//! `crates/protocol`（内核零引用；依赖方向 `kernel → env → runtime → protocol → programs`）。
//!
//! 方案 3（typed payload）：各调用域枚举（`RoomCall` 等）是带类型载荷的 variant，
//! 字段类型为语义句柄（`PieToken`/`TaskId`/`VirtAddr`）或 `Permission`/裸量；
//! `[call]` 载荷由 `derive(Envcall)` 生成 codec（`slot/pack/from_wire/call`）。
//! 返回类型经 `#[ret(T)]` 标注，derive 生成域 `*Ret` 枚举。
//!
//! **面的两层**：`pub mod` 是**全部**（`env::assembly::ALL`、`env::wire::supply::Want`、
//! `env::wire::args::VIEW` 这类路径都在），下面的 `pub use` 是**便利层**——收的是"调用点
//! 当词汇用"的那些名字（各调用域枚举、句柄、错误词汇、权限、坐标、名字…）。
//! 两族**不进**便利层：`assembly` 的格子（`Plan` / `Row` / `Spot`）与 `wire::supply` 的
//! 词汇（`Need` / `Want` / `Kind` / `At`）——它们都是通用词，平铺到 `env::` 根上只会让名字
//! 离开上下文（调用点今天是 `env::assembly::ALL`、`env::wire::supply::Need` 这两形）。

// 清单的写侧（`wire::manifest::pack`）要一段可增长的字节缓冲，故本 crate 引 `alloc`
// （读侧零分配）。`env` 仍在宿主上可编译——`alloc` 两边都有。
extern crate alloc;

pub mod assembly;
pub mod ecall;
pub mod exit;
pub mod fid;
pub mod permission;
pub mod wire;

pub use ecall::{EnvError, EnvResult, Fail, make_err};
pub use exit::{EXIT_FAULT, EXIT_OK, EXIT_PANIC, Reason};
pub use fid::{
    ChronoCall, ChronoCallRet, ControlCall, ControlCallRet, DBCN_MAX, DebugCall, DebugCallRet,
    EnvCall, HoleDir, MailCall, MailCallRet, MemoryCall, MemoryCallRet, NOTE_MAX, PieCall,
    PieCallRet, ProgramKind, RoomCall, RoomCallRet, ToleCall, ToleCallRet, UnitCall, UnitCallRet,
};
pub use permission::Permission;
pub use wire::{
    Access, Decode, FromPair, KEY_LEN, Key, Mark, NAME_LEN, Name, NameError, PAIR_LEN, Pair,
    PieToken, Policy, TaskId, TeamId, VirtAddr, Wire,
};
