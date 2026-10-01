//! 那几台"服务"（独立域）那一层的目录。
//! **每台的树同形，且每一份文件 = 一件东西（它的结构 ＋ 它的方法）**：
//! 故四台的核不同形是**有意的**：`operator` 是三件（账 ＋ operator::core::judge 判据 ＋
//! `gate` 裁决）、`principal` 是三件（账 ＋ `roster` 名册 ＋ `lineage` 谱系）、`hub` 是三件
//! （账 ＋ `ledger` 册 ＋ `league` 类→盟），而 `coalition` 的核只有盟册**一件**，就是 `core.rs`。

pub mod coalition;
pub mod hub;
pub mod operator;
pub mod principal;
