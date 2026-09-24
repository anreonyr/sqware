//! root::实现侧 — **引导域那一摊**：程序入口 ＋ "只有它读得到"的那两块账 ＋ 那圈发货循环。
//!
//! 引导域是 boot 引入的**唯一顶层域**：清单与配对块只借映进它那张表（[`boot`]）。别的域
//! 要那块载荷，走物料面（[`protocol::driver::supply`]）领——那一侧的**服务端**就是
//! [`supply`]（本域常驻的那圈发货循环；从前它与 `service.rs` 并排住在
//! 本 crate 顶层，读者会以为它也是两个装配者共用的那一件，其实只有本域用）。
//!
//! 台主（`prog-rig` / `prog-load` / `prog-again` / `prog-group`）当引导镜像时读的是同两块账，
//! 故也 `use` 这一份。

pub mod boot;
pub mod supply;
