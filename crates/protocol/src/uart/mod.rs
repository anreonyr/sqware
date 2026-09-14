//! uart — 串口驱动的**私有字节协议**：一个动词（写）+ 一条配给下来的投递孔。
//!
//! # 为什么只有一个动词
//!
//! 字节的**来路与去路本来就不对称**：
//!
//! ```text
//! 去路（写）  一问一答、带回执、背压落在设备的 THRE 上 —— 是协议
//! 来路（收）  中断驱动的事件流，驱动排空后投进一条约定的孔 —— 不是协议
//! ```
//!
//! 于是"读"没有动词：它在装配期由 root 开辟一枚投递孔，把 WRITE 副本给驱动、
//! READ 副本给 console——`docs/driver.md` §5.7「名字的配给」那条形状（一次性孔 +
//! `Accord` + `Pier`），与设备门闩走同一条下行通道。
//!
//! # 三块分工（与 `dispatch`/`console`/`doom`/`irq` 同形）
//!
//!   [`wire`]   —— 线格式：`Request`/`Status`/`MSG_LEN`，**纯函数、零依赖**；
//!   [`client`] —— 线对侧：`Uart`（连上驱动、同步写一段字节）；
//!   [`server`] —— 服务侧：`serve`（认动词、验字段）→ [`Action`]，**不做 I/O、无状态**。
//!
//! # 服务名与设备名是两笔账
//!
//! 服务名 = [`wire::SERVICE`]（`"uart"`，目录里登记的那一个）；设备名 = `serial@10000000`
//! （boot 从设备树搬来的身份，用于**登记中断线**）。同一个域**两个名字**，用途不重叠：
//! 前者给客户端找服务，后者给驱动认领自己的线。
//!
//! # 中断怎么走到这一步
//!
//! PLIC 按**线号**取持有者（`protocol::irq::server::Lines::holder`）并把线号投进它的会话孔
//! ⇒ **持有者被当场唤醒**（内核 `try_push` 唤醒站点）。持有者就是本协议的**服务侧**
//! （串口驱动域），它排空设备后把字节投给 console —— 唤醒链两跳，每跳都是一次显式唤醒。

pub mod client;
pub mod server;
pub mod wire;

pub use client::Uart;
pub use server::{Action, serve};
pub use wire::{ACK_LEN, CAP, DELIVER, HEAD, PAYLOAD_MAX, Query, SERVICE, Status};
