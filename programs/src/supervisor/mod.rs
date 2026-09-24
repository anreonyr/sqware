//! supervisor — **S 态那一档**：只有监督侧用的那几片实现，与它们的程序入口。
//!
//! 判据是特权级（唯一声明处：`env::assembly::ALL` 里这一行的 `kind`）：本目录下的**程序**都是
//! `Supervisor`（`root` / `system` 两个域，加 `supply` 那一片共用件）。
//!
//! **照实记（iii 之后那条例外没有了）**：原先 [`principal`] 与 [`coalition`] 是这条判据的两个
//! 例外（它们是 **U 态**服务，住这里只因为目录按**角色**分）。iii 把持树者 / 身份 / 结盟三枚收进
//! **编排域自己的域**（[`system`] 的 `main` 按角色分派、`scenario.rs` 的 `INNER` 给次序）⇒
//! 它们不再是程序，"特权级"在本目录里**一条例外都没有**；而"实现方跟着它那个档走"这条判据照旧
//! ——三枚内件的档就是编排域那一档。
//!
//! - **实现**：[`supply`]（引导域那圈发货
//!   循环）、[`operator`]（持树者：那棵命名树的服务，也是这台机器的**转授权中枢**——谁在树上
//!   查到一条，它就 `ship` 一枚带 `VEST` 的副本）、[`principal`]（身份服务：名册 + 谱系）、
//!   [`coalition`]（结盟服务：横向那张盟籍表——它是身份服务的**客人**）、
//!   [`system`]（编排域的实现；**板那一台** [`system::board`] 也在它里面——板线程跑在编排域
//!   的宿主线程里）。
//! - **程序入口与它那片模块同住**：`root/`（引导域：入口 ＋ 只有它读得到的那两块账）、
//!   `system/main.rs`（编排域：入口 ＋ 四个角色的分派）。**三枚内件没有入口文件**——它们的
//!   `server::serve()` 由编排域那一枚 `main` 叫起来（iii）。
//! - **共用件只有 [`service`]**：那台装配机器被**两个装配者**用（`root` 与 `system`），内含到
//!   任一方都会复制一份，故留在这里。入口样板 [`crate::entry`] 是每个程序共用的，平铺在
//!   `src/` 根。
//!
//! **表归主人**：boot 的两块账在**引导域**（[`root::boot`]）——装配者只是 `use` 它。硬件需求单
//! 在各**收方**那里（[`crate::driver::router::needs`] / [`crate::driver::uart::needs`]）。
//!
//! U 态那一档在 [`crate::user`]；**驱动与压测台按角色分档**，整块留在 [`crate::driver`] 与
//! `harness`（测具那一个 crate）。

pub mod coalition;
pub mod operator;
pub mod principal;
pub mod root;
pub mod service;
pub mod supply;
pub mod system;
