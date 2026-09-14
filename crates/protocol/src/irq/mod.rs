//! irq — 中断线协议（PLIC 驱动 ↔ 它的客户端；两端共用一份，不各写一遍）。
//!
//! # 一句话
//!
//! 客户端按**名字**认领一台设备的中断线。线号是名字的函数（名字 → 设备树节点 →
//! `interrupts` × `interrupt-parent`），由驱动自己解出来，故**报文里没有可伪造的线号**：
//! 客户端既报不出、也抢不走一条不属于它的线（`docs/driver.md` §12 甲）。
//!
//! # 三块分工（与 `dispatch`/`console` 同形）
//!
//!   [`wire`]   —— 线格式：`Query`/`Ack`/`Refused`/`CAP`，**纯函数、零依赖**；
//!   [`client`] —— 线对侧：`Line`（连上驱动、登记 / 写属主 / 委托写权）；
//!   [`server`] —— 服务侧：线表 [`Lines`]（名字 → 线号 + 属主 + 实例）。
//!
//! 驱动本身住在 `prog-plic` 域（`programs/src/bin/supervisor/plic.rs`）：寄存器
//! （`claim`/`complete`/开关线）与设备树解析是**设备的活**，留在那一层；表与判据才是
//! 协议——故 [`Lines::new`] 收的是驱动从树里解好的行，本 crate 不认识 `fdt`。

pub mod client;
pub mod server;
pub mod wire;

pub use client::Line;
pub use server::Lines;
pub use wire::{ACK_LEN, Ack, CAP, LINE_LEN, Query, Refused, SERVICE, TEXT};
