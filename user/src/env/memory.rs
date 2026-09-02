//! Memory 域：`MemoryCall::*` 转发。

use ubi::{MemoryCall, UArgs, UResult, Ucall, UcallBuilder};

use crate::PAGE_SIZE;

pub fn allocate(size: usize) -> UResult<usize> {
    let size = size.max(1).next_multiple_of(PAGE_SIZE);
    let args = UArgs {
        a0: size,
        ..UArgs::default()
    };
    let (v0, _) = UcallBuilder::new(Ucall::Memory(MemoryCall::Allocate))
        .args(args)
        .call()?;
    Ok(v0)
}

pub fn deallocate(addr: usize, size: usize) -> UResult<()> {
    let size = size.max(1).next_multiple_of(PAGE_SIZE);
    let args = UArgs {
        a0: addr,
        a1: size,
        ..UArgs::default()
    };
    let _ = UcallBuilder::new(Ucall::Memory(MemoryCall::Deallocate))
        .args(args)
        .call()?;
    Ok(())
}

/// `at = None` 走窗口自选，`Some(addr)` 走固定地址。
pub fn mmap(size: usize, at: Option<usize>) -> UResult<usize> {
    let size = size.max(1).next_multiple_of(PAGE_SIZE);
    let args = UArgs {
        a0: size,
        a2: at.unwrap_or(0),
        ..UArgs::default()
    };
    let (v0, _) = UcallBuilder::new(Ucall::Memory(MemoryCall::Mmap))
        .args(args)
        .call()?;
    Ok(v0)
}

pub fn munmap(addr: usize, size: usize) -> UResult<()> {
    let size = size.max(1).next_multiple_of(PAGE_SIZE);
    let args = UArgs {
        a0: addr,
        a1: size,
        ..UArgs::default()
    };
    let _ = UcallBuilder::new(Ucall::Memory(MemoryCall::Munmap))
        .args(args)
        .call()?;
    Ok(())
}

pub fn mprotect(addr: usize, size: usize, flags: u64) -> UResult<()> {
    let size = size.max(1).next_multiple_of(PAGE_SIZE);
    let args = UArgs {
        a0: addr,
        a1: size,
        a2: flags as usize,
        ..UArgs::default()
    };
    let _ = UcallBuilder::new(Ucall::Memory(MemoryCall::Mprotect))
        .args(args)
        .call()?;
    Ok(())
}
