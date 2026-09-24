#![no_std]
//! 环境调用 ABI（env）：**过线的那些东西**——U 态任务与 S 态域任务共用的调用封装
//! （slot 编码 + 载荷 codec + 线类型 + 发起骨架），加门闩权限（位掩码 + 两族视图）。
//!
//! **不含服务目录协议**（`Request`/`Reply`/`MSG_LEN`）——那是纯用户态协议，住在
//! `crates/protocol`（内核零引用；依赖方向 `kernel → env → runtime → protocol → programs`）。
//!
//! **也不含"装机的账"**（照实记）：装配单、供给词汇、坐标、开机那三笔借映块原先也住这里
//! （分居 `assembly.rs` 与 `wire/{supply,key,args,pair,manifest}.rs`），而它们与本 crate 的
//! 共同点只有"宿主与 riscv 都编得过"——那是**编译得了**，不是**同一件事**。现在它们住
//! `crates/plan`（判据见那边 `lib.rs` 的头注）：`env` 是**过线的**，`plan` 是**装机的**，
//! 方向单向 `plan → env`。
//!
//! 方案 3（typed payload）：各调用域枚举（`RoomCall` 等）是带类型载荷的 variant，
//! 字段类型为语义句柄（`PieToken`/`TaskId`/`VirtAddr`）或 `Permission`/裸量；
//! `[call]` 载荷由 `derive(Envcall)` 生成 codec（`slot/pack/from_wire/call`）。
//! 返回类型经 `#[ret(T)]` 标注，derive 生成域 `*Ret` 枚举。
//!
//! **面的两层**：`pub mod` 是**全部**，下面的 `pub use` 是**便利层**——收的是"调用点当词汇
//! 用"的那些名字（各调用域枚举、句柄、错误词汇、权限、名字…）。`Plan` / `Row` / `Spot`
//! 那一族与 `Need` / `Want` / `Key` / `Pair` 那两族**不在本 crate 里**（见上），
//! 它们的便利层在 `plan`。
//!
//! **本 crate 不引 `alloc`**（照实记）：从前引它是为了 `wire::manifest::pack` 那段可增长的
//! 字节缓冲，而清单随 `plan` 走了——故 `env` 今天**一个字节都不分配**。

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
pub use permission::{Access, Permission, Policy};
pub use wire::{
    Decode, FromPair, Mark, NAME_LEN, Name, NameError, PieToken, TaskId, TeamId, VirtAddr, Wire,
};
