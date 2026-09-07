//! Spawnee — 可 spawn 的镜像编目（编译期枚举，过渡态）。
//!
//! 在没有文件系统的现状下，ELF 字节只能 `include_bytes!` 内嵌内核镜像。本枚举把
//! 「哪些镜像能 spawn」编目成编译期常量（过渡态：文件系统出现后整个枚举弃用，
//! spawn 的字节来源改由 FS 命名空间提供）。
//!
//! 分层：本文件只放 `Spawnee` 枚举 + `impl Wire`（用户态能发判别值，拿不到字节）。
//! `Spawnee::elf()`（`include_bytes!(env!("USER_*"))` 取字节）只能在**内核侧**实现
//! ——`USER_*` 环境变量只在 `kernel/build.rs` 定义，`ubi` 编译时无此 env。

use crate::wire::{Decode, Wire};

/// 镜像判别（编译期定死；新增镜像须同时加变体 + `kernel` 侧 `elf()` 一支）。
#[repr(usize)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Spawnee {
    /// lisp 解释器（`user/src/bin/lisp.rs`）。
    Lisp = 0,
    /// shell 命令解释器（`user/src/bin/shell.rs`）。
    Shell = 1,
    /// BACK 位 demo（`user/src/bin/back.rs`）。
    Back = 2,
    /// narrow demo（`user/src/bin/narrow.rs`）。
    Narrow = 3,
    /// sire 溯源 demo（`user/src/bin/sire_demo.rs`）：读自己的 sire()/self_id()。
    Sire = 4,
}

impl Spawnee {
    /// 镜像名（诊断用）。
    pub const fn name(self) -> &'static str {
        match self {
            Spawnee::Lisp => "lisp",
            Spawnee::Shell => "shell",
            Spawnee::Back => "back",
            Spawnee::Narrow => "narrow",
            Spawnee::Sire => "sire",
        }
    }
}

impl Wire for Spawnee {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        s[*i] = *self as usize;
        *i += 1;
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        let v = *s.get(*i).ok_or(Decode::Overflow)?;
        *i += 1;
        match v {
            0 => Ok(Spawnee::Lisp),
            1 => Ok(Spawnee::Shell),
            2 => Ok(Spawnee::Back),
            3 => Ok(Spawnee::Narrow),
            4 => Ok(Spawnee::Sire),
            _ => Err(Decode::Invalid),
        }
    }
}
