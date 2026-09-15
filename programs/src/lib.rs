#![no_std]
//! programs — 镜像里装载的程序集合（每个 `src/bin/` 一个）。
//!
//! 本 lib 现在只有一件共享物：`entry`（`_start` + panic 处理，各程序共用）。
//! 设备侧（UART / PLIC / 设备树）与终端渲染都随旧树一起清了（tag `proto-v1-baseline`）
//! ——这一版没有驱动、没有服务，跨域说话走 `env` 的调试面。
//!
//! `bin/` 里只有两个：`prog-root`（装配者）与 `prog-echo`（调试回显）。

pub mod entry;
