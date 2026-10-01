//! service — **那几台"服务"**（独立域）那一层的目录。
//!
//! **每台的树同形，且每一份文件 = 一件东西（它的结构 ＋ 它的方法）**：
//! ```text
//!   core.rs / core/   核（纯）：一件就一份文件，两件以上才进一层目录
//!   serve/            那一枚线程（服务侧 / 持树侧）＋ 它叫的那几手
//!   bridge.rs         适配·装配侧（装配者手里那侧）——没有的台就没有这个文件
//!   claim.rs          载体（本域表上那几枚孔）——用"门牌自带回信孔"的台就没有这个文件
//!   main.rs  mod.rs  program.rs   入口 / 该台的自述 / 装配声明
//! ```
//! 故四台的核不同形是**有意的**：`operator` 是三件（账 ＋ [`operator::core::judge`] 判据 ＋
//! `gate` 裁决）、`principal` 是三件（账 ＋ `roster` 名册 ＋ `lineage` 谱系）、`hub` 是三件
//! （账 ＋ `ledger` 册 ＋ `league` 类→盟），而 `coalition` 的核只有盟册**一件**，就是 `core.rs`。

pub mod coalition;
pub mod hub;
pub mod operator;
pub mod principal;
