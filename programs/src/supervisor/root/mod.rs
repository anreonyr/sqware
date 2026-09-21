//! root::实现侧 — **引导域那一摊**：程序入口，与"只有它读得到"的那两块账。
//!
//! 引导域是 boot 引入的**唯一顶层域**：清单与配对块只借映进它那张表（[`boot`]）。别的域
//! 要那块载荷，走物料面（[`protocol::driver::supply`]）领。
//!
//! 台主（`prog-rig` / `prog-load` / `prog-again` / `prog-group`）当引导镜像时读的是同两块账，
//! 故也 `use` 这一份。

pub mod boot;
