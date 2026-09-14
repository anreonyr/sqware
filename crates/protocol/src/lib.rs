#![no_std]
//! protocol — 用户态协议层（**内核不依赖本 crate**）。
//!
//! 三方分工按**依赖方向**，不按行数：
//!   `env`      = ABI：内核与用户态**都要**的线格式与调用骨架（`EnvCall`/`wire`/`Permission`）；
//!   `runtime`  = 机制：把 ABI 落成可用的运行时（`env` 薄转发 + `core` 组合封装）；
//!   `protocol` = 语义：只跑在用户态、且**只对某个协议有意义**的东西。
//!
//! 依赖：`protocol → runtime → env`（单向，无环）。
//!
//! 为什么要有这一层：目录协议原先住在 `crates/env` 里，于是"内核也依赖的 ABI crate"
//! 里躺着 284 行内核永远读不到的协议（全仓内核引用数 = 0，实测），"这是用户态的东西"
//! 这句话从结构上被抹掉了。独立成 crate 之后，**内核不知道目录协议**由编译期依赖
//! 保证：`kernel/Cargo.toml` 里没有 `protocol`，想引用也引用不到。
//!
//! 为什么现在是 `→ runtime` 而不是 `→ env`：协议的用户态实现要用**机制**
//! （门闩 `HolePie`、回信通道 `Channel`），而机制在 `runtime`。这一条边就是
//! "机制在运行时、语义在协议"这句话的编译期形态。
//!
//! # 四个协议，同一个形状
//!
//! ```text
//! dispatch   服务目录（名字 → 预约者 + 实例）     `docs/dispatch.md`
//! console    控制台（唯一读 UART 的服务）         `docs/console.md`
//! doom       他杀（`kill`：一条请求 + 一字节回执）`docs/root.md` §5.1
//! irq        中断线（按名字认领一条线）           `docs/driver.md` §12 甲
//! ```
//!
//! 每个协议照**同一个形状**摆四块，`mod.rs` 只做「文档 + 转出」：
//!
//! ```text
//! mod.rs     协议是什么、为什么这么摆；`pub mod` + `pub use`（旧进口路径因此照旧）
//! wire.rs    线格式：动词、报文长度、编解码、回执码、SERVICE——纯函数、零依赖
//! client.rs  线对侧：会话对象
//! server.rs  服务侧：表 / 状态机——**不做 I/O**，设备与内核效果靠注入或留在适配层
//! ```
//!
//! `server.rs` 有两种已被验证的形态，按"谁是设备"选：目录与 console 把核心状态机整个
//! 搬进来（内核动作经 `Release` 注入）；irq 只搬**表**，寄存器与设备树留在驱动侧
//! （`prog-plic`）；doom 只搬**判据**（[`doom::collect`]），线程骨架与回执的推送留在
//! root 域。判据是同一条：**协议管"是什么、按什么判"，程序管"怎么装"**。

extern crate alloc;

pub mod console;
pub mod dispatch;
pub mod doom;
pub mod irq;
// `uart` 是**设备面**的私有协议（一台串口、一个动词），故不进上面"四个协议"那张表：
// 那四个是系统语义（名字 / 终端 / 他杀 / 中断线），它是"把字节交给设备"。
pub mod uart;
