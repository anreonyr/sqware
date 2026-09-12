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

/// **临时探针**：读帧池水位 `(pagemeta 在手帧数 = 真相, freelist 走链帧数)`。
///
/// 用途只有一个：让「已知量 alloc/free 前后各读一次」的闭环校准能在**同一个
/// 进程内**完成，不必靠任务生灭去触发内核侧打印。判据立住后连同
/// `MemoryCall::Watermark` 一起删。
pub fn watermark() -> EnvResult<(usize, usize)> {
    let r = MemoryCall::Watermark {
        kinds: VirtAddr::new(0),
    }
    .call()?;
    match r {
        MemoryCallRet::Watermark(w) => Ok(w),
        _ => unreachable!(),
    }
}

/// **临时探针**：`live` = 累计分配 − 累计释放（帧数）。守恒口径：它加上
/// "非 `deallocate` 路径释放的帧"恒等于池内已交付帧数。
pub fn live_frames() -> EnvResult<usize> {
    let r = MemoryCall::LiveFrames.call()?;
    match r {
        MemoryCallRet::LiveFrames(v) => Ok(v),
        _ => unreachable!(),
    }
}

/// **临时探针**：`merge_block` 总计数 `(ok, bound 拒, meta 拒, chain 拒)`。
pub fn merge_census() -> EnvResult<(usize, usize, usize, usize)> {
    let r = MemoryCall::MergeCensus { power: usize::MAX }.call()?;
    match r {
        MemoryCallRet::MergeCensus(v) => Ok((
            v & 0xffff,
            (v >> 16) & 0xffff,
            (v >> 32) & 0xffff,
            (v >> 48) & 0xffff,
        )),
        _ => unreachable!(),
    }
}

/// **临时探针**：某 order 的拒绝分布 `(meta 拒, chain 拒)`。
pub fn reject_by_power(power: usize) -> EnvResult<(usize, usize)> {
    let r = MemoryCall::MergeCensus { power }.call()?;
    match r {
        MemoryCallRet::MergeCensus(v) => {
            Ok(((v & 0xffff_ffff) as usize, ((v >> 32) & 0xffff_ffff) as usize))
        }
        _ => unreachable!(),
    }
}

/// **临时探针**：水位 + **逐类在册帧数**（`trap=12 stack=85 table=420 …`）。
///
/// 分类水位是判漏该用的表：只盯池总量，任何一类在漏都长一个样；逐类看才能
/// 直接指到漏的是哪一类。缓冲由调用方给（用户态栈上），内核按字节写回。
pub fn watermark_kinds(buf: &mut [u8]) -> EnvResult<(usize, usize)> {
    let r = MemoryCall::Watermark {
        kinds: VirtAddr::new(buf.as_mut_ptr() as usize),
    }
    .call()?;
    match r {
        MemoryCallRet::Watermark(w) => Ok(w),
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
