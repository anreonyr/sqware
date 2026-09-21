//! supervisor — **S 态那一档**：只有监督侧用的那几片实现，与它们的程序入口。
//!
//! 判据是特权级（唯一声明处：`kernel/build.rs::INITRD_BINS`）：本目录下**除 [`principal`]
//! 与 [`coalition`] 外**都是 `Supervisor`。**照实记**：那两条（身份服务、结盟服务）是 **U 态**
//! ——它们不持有、不授予、不解释任何 Pie，只读写自己那几张表，故不需要 S 态；它们住这里是因为
//! 目录按**角色**分（同名服务的判定与接口住 `crates/protocol/src/<名>/`），不是因为特权级。
//!
//! - **实现**：[`supply`]（引导域那圈发货
//!   循环）、[`operator`]（持树者：那棵命名树的服务，也是这台机器的**转授权中枢**——谁在树上
//!   查到一条，它就 `ship` 一枚带 `VEST` 的副本）、[`principal`]（身份服务：名册 + 谱系）、
//!   [`coalition`]（结盟服务：横向那张盟籍表——它是身份服务的**客人**）、
//!   [`system`]（编排域的实现；**板那一台** [`system::board`] 也在它里面——板线程跑在编排域
//!   的宿主线程里）。
//! - **程序入口与它那片模块同住**：`root/`（引导域：入口 ＋ 只有它读得到的那两块账）、
//!   `operator/main.rs`、`principal/main.rs`、`coalition/main.rs`、`system/main.rs`。
//! - **共用件只有 [`service`]**：那台装配机器被**两个装配者**用（`root` 与 `system`），内含到
//!   任一方都会复制一份，故留在这里。入口样板 [`crate::entry`] 是每个程序共用的，平铺在
//!   `src/` 根。
//!
//! **表归主人**：boot 的两块账在**引导域**（[`root::boot`]）——装配者只是 `use` 它。硬件需求单
//! 在各**收方**那里（[`crate::driver::router::needs`] / [`crate::driver::uart::needs`]）。
//!
//! U 态那一档在 [`crate::user`]；**驱动与压测台按角色分档**，整块留在 [`crate::driver`] 与
//! [`crate::stress`]。

pub mod coalition;
pub mod operator;
pub mod principal;
pub mod root;
pub mod service;
pub mod supply;
pub mod system;
