#![no_std]
//! protocol — **口**：怎么开口。
//!
//! 这一层只剩**碰内核的那几件**：客侧那几手（`*/client.rs`）、十件手的身体（`session/call.rs`）、
//! 绑真手的构造（`system/*/mod.rs`；两张对照表随它们产出的 `Fail` 落进「约」的 `core`）。
//!
//! **那句话本身**（正文、形、据、账、不碰内核的适配）住 **`crates/contract`（「约」）**——
//! 全树的总说明在那份 `lib.rs`；判据一条：**`contract` 的依赖里没有本 crate**。
//!

// 码头的泊位是**一张可增长的账**（`session::core::Quay`）：条数由调用方按路数决定，
// 故本 crate 引 `alloc`（与 `env`/`runtime` 同款；备不下时由 `Vec::try_reserve` 如实报
// `Seat::Full`，不 panic）。
extern crate alloc;

#[macro_use]
mod reserve_reads;

// 码表宏自己一份源：**「约」各家**同读这一份（`frame.rs` 那几份住在「约」里，`crate` 指的就是
// 「约」的根）。
//
// 三件共享件搬进 `crates/contract`（「约」）之后 `#[macro_use]` 仍要——它把那个宏带进
// **本 crate 后面那些模块**的作用域（宏的可见性按正文先后），本 crate 那几处
// `crate::fail_codes! { … }` 靠它；**出 crate 那一份**从此是 `contract::fail_codes!`。
// 转出「约」那一份**模块**（宏也在里面）：`crate::fail_codes::OK` 与
// `crate::fail_codes! { … }` 两条路都解析。
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
// ⇒ 调用点一处不改。
pub use contract::{frame, id};
pub mod session;
pub mod system;

// 依赖先留着：`env` 与 `runtime` 是地板，第一条协议操作出现时立刻要用。
// （本文件自己只有：`reserve_reads` 那个模块（宏在里面）、上面那几条转出、三个 `pub mod`
// （`driver` / `session` / `system`）——`env` / `runtime` 只出现在**宏体**里，由调用宏的那些模块
// 去用 ⇒ `cargo` 若在**本文件这一格**报 unused dependency，是预期噪音。）
