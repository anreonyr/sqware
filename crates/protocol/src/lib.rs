#![no_std]
//! protocol — **口**：怎么开口。
//!
//! 这一层只剩**碰内核的那几件**：客侧那几手（`*/client.rs`）、十件手的身体（`session/call.rs`）、
//! 绑真手的构造与两张对照表（`system/*/call.rs`）。
//!
//! **那句话本身**（正文、形、据、账、不碰内核的适配）住 **`crates/contract`（「约」）**——
//! 全树的总说明在那份 `lib.rs`；判据一条：**`contract` 的依赖里没有本 crate**。
//!

// 码头的泊位是**一张可增长的账**（`session::core::Quay`）：条数由调用方按路数决定，
// 故本 crate 引 `alloc`（与 `env`/`runtime` 同款；备不下时由 `Vec::try_reserve` 如实报
// `Seat::NoRoom`，不 panic）。
extern crate alloc;

// ── 术语与它的两支宏 ───────────────────────────────────────
//
// 两支宏只做一件事：**把一份身体按格／按表铺开**——一处裁决都不放。术语全部从地板取
// （`env::fid::PieCall::Reserve` 的三格名 `vestor` / `owner` / `mark`、线上答话那一格），
// 不自造。住 crate 根是因为板、树、线、货四家都要用，而本仓**不为一个形状新开文件**。

/// `Reserve` 的三格：**一个调用的三个事实**，按格展开——一格一个读出。
///
/// 三格的读法只此一份（`mail::reserve` 那一问 + 两个哨兵）：`owner == 0`（引导期那批设备
/// 门闩）不算"谁开的"；记号答不出就报 `None`。调用点只给**格名**（`vestor` / `owner` /
/// `mark`）与**自己那一侧的名字**，故"三格是一组、三个名字等长"在调用处一眼可见——
/// 原先板与树各写一份 `probe`5 / `opened_by`9 / `mark_of`7，不等长本身就是信号。
///
/// `$vis` 那一格是给**共享体与领域名分家**用的：身体住 `session::call`，名字由调用点给。
macro_rules! reserve_reads {
    ($(#[$meta:meta])* $vis:vis fn $name:ident($arg:ident) => vestor $(;)?) => {
        $(#[$meta])*
        $vis fn $name($arg: env::PieToken) -> Option<env::TaskId> {
            runtime::env::mail::reserve($arg)
                .ok()
                .map(|(vestor, _owner, _mark)| vestor)
        }
    };
    ($(#[$meta:meta])* $vis:vis fn $name:ident($arg:ident) => owner $(;)?) => {
        $(#[$meta])*
        $vis fn $name($arg: env::PieToken) -> Option<env::TaskId> {
            match runtime::env::mail::reserve($arg) {
                Ok((_vestor, owner, _mark)) if owner.get() != 0 => Some(owner),
                _ => None,
            }
        }
    };
    ($(#[$meta:meta])* $vis:vis fn $name:ident($arg:ident) => mark $(;)?) => {
        $(#[$meta])*
        $vis fn $name($arg: env::PieToken) -> Option<env::Mark> {
            match runtime::env::mail::reserve($arg) {
                Ok((_vestor, _owner, mark)) => Some(mark),
                _ => None,
            }
        }
    };
}

// 码表宏自己一份源：**「约」与各宿主靶**同读这一份（靶不依赖本 crate，见那份文件的照实记）。
//
// 三件共享件搬进 `crates/contract`（「约」）之后 `#[macro_use]` 仍要——它把那个宏带进
// **本 crate 后面那些模块**的作用域（宏的可见性按正文先后），本 crate 那几处
// `crate::fail_codes! { … }` 靠它；**出 crate 那一份**从此是 `contract::fail_codes!`。
// 转出「约」那一份**模块**（宏也在里面）：`crate::fail_codes::OK` 与
// `crate::fail_codes! { … }` 两条路都解析——**宿主靶也一样**（它把调用宏的那几份
// `call.rs` 逐字编进去，`crate` 指靶，而靶自己也有这份模块）。
pub use contract::fail_codes;

/// **答话那一格的"没失败"**（0）——全协议**一个号**：六家（principal / coalition / operator /
/// board / line / supply）与驱动各自那几族（如 `programs::driver::rtc`）共用。
///
/// 定义在 [`contract::fail_codes`] 那一份源里（`fail_codes!` 的第二个参数就是它）；这里把它**转出
/// crate**：`fail_codes` 自己是有意私有的（出 crate 的只有那个宏），而驱动那一侧的具体协议住在
/// `programs` 里，要读这一格只能从 crate 根进来。
///
/// **各家的失败码不共用**（同一个概念在两家是别的号，见各族 `frame.rs` 的注）——共用的只有
/// "没失败"那一格。
pub use contract::fail_codes::OK;

pub mod driver;
// 转出「约」那两件：`crate::frame` / `crate::id` 与 `protocol::frame` / `protocol::id` **照旧解析**
// ⇒ 调用点一处不改（宿主靶那几处 `#[path]` 改指新家，见 `crates/protocol-case`）。
pub use contract::{frame, id};
pub mod session;
pub mod system;

// 依赖先留着：`env` 与 `runtime` 是地板，第一条协议操作出现时立刻要用。
// （本文件自己只有：下面那条 `reserve_reads!` 宏、`mod fail_codes`、五个 `pub mod` 与那两条
// 编译期断言——`env` / `runtime` 只出现在**宏体**里，由调用宏的那些模块去用 ⇒ `cargo` 若在
// **本文件这一格**报 unused dependency，是预期噪音。）

