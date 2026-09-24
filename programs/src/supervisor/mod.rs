//! supervisor — **S 态那一档**：只有监督侧用的那几片实现，与它们的程序入口。
//!
//! 判据是特权级（唯一声明处：`env::assembly::ALL` 里这一行的 `kind`）：本目录下的**程序**都是
//! `Supervisor`（`root` / `system` 两个域）。
//!
//! **目录按程序分**（另一把尺子，见 `lib.rs` 的"谁在说话"：实现方跟着**用它那个程序所在的
//! 档**走）：
//!
//! - [`root`] —— 引导域那一摊：入口 ＋ [`root::boot`]（只有它读得到的那两块账）＋
//!   [`root::supply`]（那圈发货循环；全仓只有本域用它）；
//! - [`system`] —— 编排域那一摊：入口 ＋ 装配单（`scenario.rs`）＋ 机器（`server.rs`）＋
//!   住本域的那几枚线程（编排者自己 ＋ [`system::board`] 板 ＋ [`system::operator`] 持树者 ＋
//!   [`system::principal`] 名册 ＋ [`system::coalition`] 盟册）。
//!
//! **这两摊之外只摆共用件**：[`service`] —— 那台装配机器被**两个装配者**用（`root` 与
//! `system`），内含到任一方都会复制一份，故留在这里。入口样板 `entry` 是每个程序共用的，
//! 平铺在 `src/` 根。
//!
//! **照实记（iii 之后那条例外没有了；这一刀目录也跟上了）**：原先 [`system::principal`] 与
//! [`system::coalition`] 是这条判据的两个例外（它们是 **U 态**服务，住这里只因为目录按**角色**
//! 分）。iii 把持树者 / 身份 / 结盟三枚收进**编排域自己的域**（[`system`] 的 `main` 按角色分派、
//! `scenario.rs` 的 `INNER` 给次序）⇒ 它们不再是程序、"特权级"在本目录里**一条例外都没有**；
//! 用户裁定「搬」之后它们与板一样住 [`system`] 之下——**目录与两把尺子都对齐了**。
//!
//! **表归主人**：boot 的两块账在**引导域**（[`root::boot`]）——装配者只是 `use` 它。硬件需求单
//! 在各**收方**那里（[`crate::driver::router::needs`] / [`crate::driver::uart::needs`]）。
//!
//! U 态那一档在 [`crate::user`]；**驱动与压测台按角色分档**，整块留在 [`crate::driver`] 与
//! `harness`（测具那一个 crate）。

pub mod root;
pub mod service;
pub mod system;
