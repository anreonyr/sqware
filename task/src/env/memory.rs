//! Memory 域：`MemoryCall::*` 转发。
//!
//! 方案 3（typed payload）：参数经 `VirtAddr` 包装，构造即类型安全；返回
//! `MemoryCallRet`，`Allocate/Mmap` 蒸馏出 VA。

use env::{EnvResult, MemoryCall, MemoryCallRet, VirtAddr};

use crate::PAGE_SIZE;

pub fn allocate(size: usize) -> EnvResult<usize> {
    let size = size.max(1).next_multiple_of(PAGE_SIZE);
    let r = MemoryCall::Allocate { size }.call()?;
    match r {
        MemoryCallRet::Allocate(va) => Ok(va.get()),
        _ => unreachable!(),
    }
}

pub fn deallocate(addr: usize, size: usize) -> EnvResult<()> {
    let size = size.max(1).next_multiple_of(PAGE_SIZE);
    let r = MemoryCall::Deallocate {
        addr: VirtAddr::new(addr),
        size,
    }
    .call()?;
    match r {
        MemoryCallRet::Deallocate(()) => Ok(()),
        _ => unreachable!(),
    }
}

/// `at = None` 走窗口自选，`Some(addr)` 走固定地址。
pub fn mmap(size: usize, at: Option<usize>) -> EnvResult<usize> {
    let size = size.max(1).next_multiple_of(PAGE_SIZE);
    let r = MemoryCall::Mmap {
        size,
        at: VirtAddr::new(at.unwrap_or(0)),
    }
    .call()?;
    match r {
        MemoryCallRet::Mmap(va) => Ok(va.get()),
        _ => unreachable!(),
    }
}

pub fn munmap(addr: usize, size: usize) -> EnvResult<()> {
    let size = size.max(1).next_multiple_of(PAGE_SIZE);
    let r = MemoryCall::Munmap {
        addr: VirtAddr::new(addr),
        size,
    }
    .call()?;
    match r {
        MemoryCallRet::Munmap(()) => Ok(()),
        _ => unreachable!(),
    }
}

pub fn mprotect(addr: usize, size: usize, flags: u64) -> EnvResult<()> {
    let size = size.max(1).next_multiple_of(PAGE_SIZE);
    let r = MemoryCall::Mprotect {
        addr: VirtAddr::new(addr),
        size,
        flags,
    }
    .call()?;
    match r {
        MemoryCallRet::Mprotect(()) => Ok(()),
        _ => unreachable!(),
    }
}
