//! lodger::实现侧 — **房客**：占住一条线、**直接死**（不说再见）。
//!
//! `main.rs` 是它的入口（bin）。本模块只把 [`needs`] 交给 lib：那张需求单是**收方自己开的**，
//! 装配者只是照它开单，故两边必须看同一张表（与 `driver/uart`、`driver/router` 同一个形状）。

pub mod needs;
