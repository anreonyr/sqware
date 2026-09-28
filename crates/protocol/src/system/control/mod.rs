//! Control Protocol — **Service 的存在与生命周期**：系统里有什么 Service，它们处于什么生命状态。
//!
//! ```text
//!   mint    造一个 Service（按名字：建域 + 产它的代表线程，恒产未放行）
//!            —— 镜像由**持表那一侧**从清单里取；**帧里不带镜像**
//!   start   放行 + 等就绪（有通道的那条顺带逐条认领）
//!   stop    下令收掉（**下令即回**，不等它收完）
//!   state   这一条此刻处于哪个生命阶段
//! ```
//!
//! 四手是**最终的生命周期 API**；[`Face`] 是对外那一面。
//!
//! # 不在这里的（实现侧机制或组合策略）
//!
//! ```text
//!   enlist       Server 内部表维护（登记一行）
//!   spawn        具体创建实现（统一收口为 mint 语义）
//!   until        观察实现（问 → 等 → 问）
//!   observe      观察实现（盯它一眼）
//!   wait_last    Server 收场策略
//!   restart      stop + mint + start
//!   replace      生命周期组合
//!   supervise    Server 策略
//!   dependency   Assembly / Server 编排策略
//!   connect / wire   通信与装配细节
//! ```
//!
//! **不要为了覆盖这些函数而扩张协议 ABI。**
//!
//! # 镜像从哪来：`build` 不拷字节（本轮的关键一条）
//!
//! 内核 `UnitCall::Build` 的注（`crates/env/src/fid.rs`）写着：**镜像字节不被拷走**——内核按
//! ELF 段**现读** `elf` 那几页，故那段区间在调用期间必须一直映射着。配
//! [`manifest::Entry`](env::manifest::Entry) 的 `elf: &'a [u8]`（**借用 initrd 那块 blob 的
//! 切片**）⇒ 结论只有一条：
//!
//! ```text
//!   "程序从哪来" = "在哪给你一段 &[u8]"     不是"把字节搬过哪条路"
//! ```
//!
//! 于是 `mint` 这一问线上**只有名字**：持表那一侧（它有清单）自己去取那段字节；来源分几档
//! （initrd / 文件系统…）是**装配支撑面**的事，不是生命周期这一轴的语义。把字节塞进帧里，
//! 等于给一条本地调用强加一次多余的编解码，且与"现读"正相反。
//!
//! # 可达性由入口决定，不由域决定（照实记）
//!
//! "control 住在编排域里"与"别的域能不能拿到它的 [`Face`]"是两件不相干的事：跨域会话是**既有**
//! 能力（`board` 那枚线程就住在编排域里，`harness/src/probe_bound.rs` 是另一个域，照常开会话
//! 一问一答）。缺的只是**入口怎么递出去**。今天有两条现成的路：
//!
//! ```text
//!   装配期直授   装配者把 control 那枚入口随配给/转授给指定域
//!   上树         control 把入口挂到 /sys/control（树里已有 /sys/principal、/sys/coalition 两处先例）
//!                ⇒ 任何走到树的任务 `operator::Face::entry` 一查就有 ⇒ [`Face::of`] 直接成立
//! ```
//!
//! 上树那一条最干净：取面方式与 principal / coalition **逐字同形**——**今天走的就是它**：
//! 编排域主线程在整表起完之后把它挂到 `/sys/control`（`programs/src/system/control/mount.rs`），
//! 而"铸入口那一枚必须长命"那一格由它此后进监督那一趟满足（那条挂载路原先死在这里，原委见
//! `programs/src/system/Assembly::supervise` 的照实记）。真客人是 `harness/src/probe_control.rs`。
//!
//! # 已知边界（照实写，不是待办）
//!
//! - **失败域只有五格**：`Unknown` / `BadImage` / `Full` / `NotReady` / `Bad`。前四格逐格对
//!   编配侧的 `programs/src/system/core.rs` 那四格（"调用方接下来干什么"），`Bad` 是本端
//!   读不懂那一格（表外）。
//! - **`Wait` 是这一趟的预算**：`AtMost(n)` 表示"这一趟总共至多花 n 毫秒"，不是每一趟内部重试
//!   各自重新获得 n。**但它是"额度"不是"时限"**（独立复核指出，此处照实写）：额度按重试**逐次
//!   递减**，**不含单次往返的耗时**，而推出去那一步用的是一直等（单槽满则在门外等）——故
//!   `AtMost(n)` 不构成"这一趟一定在 n 毫秒内返回"的硬保证。今天这四手**不重试**，
//!   风险落在 `operator::Face::entry` / `Face::room` 那一族（它们的照实记写了同一句）。
//! - **状态与实例坐标是两件事**：`State::Dead` 与"上一个实例的坐标还在"并存是合法的
//!   （"起过、现在死了"）——本协议的 `state()` 只读前者。
//! - **这一面上了树，但门禁只有"已绑身份"那一格**：`/sys/control` 那一格是 `Permit::Unset`
//!   （与 `/sys/principal` / `/sys/coalition` 同一格），故**任何已绑身份的域**都取得回入口，
//!   进而 `mint` / `start` / `stop` 装配表里任意一台。这不是新开的口子（`doom` 同样没有门禁），
//!   但它是这一面今天的口径，照实写在这里。要收，收的是那一格的 `Permit`（`operator` 那一侧），
//!   不是本协议的形状。

pub mod client;
pub mod frame;

pub use client::{Face, BERTH};
pub use frame::{Fail, State, ASK_MARK, BACK, LINK};
