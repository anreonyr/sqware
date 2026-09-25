//! system — **正文已搬进「约」**（`crates/contract/src/system/mod.rs`）。
//!
//! 这里只剩**碰内核的那几件**（客侧那几手、十件手的身体、绑真手的构造与两张对照表）——
//! 判据见 `crates/protocol/src/lib.rs` 与 `crates/contract/src/lib.rs`。


//! **它容纳那四套协议**（[`board`] / [`operator`] / [`principal`] / [`coalition`]——用户裁定）：
//! 判据是"**谁住编排域**"。iii 之后这四套的落地都是**编排域里的线程**（板线程 ＋ 持树者 /
//! 名册 / 盟册），而"**要找服务得先有目录**——今天那本目录就是 `board`"这句也写在本正文里。
//! 故协议树与实现树（`programs/src/system/`）**同形**：编排那三件
//! （[`core`] / [`desk`] / [`grant`]）与这四套同一份屋顶。
//!
//! **照实记（原先它们住顶层）**：`operator` / `principal` / `coalition` 曾与 [`crate::system`]
//! 平级（`crates/protocol/src/{operator,principal,coalition}/`）。**被否的那条读法**是
//! "协议树按'谁在说话'分、不该镜像实现树"——用户裁的是前者，故搬进来了。

pub mod board;
pub mod coalition;
pub mod operator;
pub mod principal;

// **判定的三件已搬进「约」**（`crates/contract`）——这里**转出**：`crate::system::core` 与
// `protocol::system::desk::…` 照旧解析，调用点一处不改。正文（那句话是什么）暂时留在本文件。
pub use contract::system::{core, desk, grant};

pub use crate::system::core::{Fail, Ready, Reaped, Watch};
pub use crate::system::desk::{Announce, Service, Slot, State, Table};
