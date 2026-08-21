#![no_std]
//! 用户态系统调用封装（U-mode → S-mode 环境调用）。
//!
//! 机制层（`UArgs`/`UError`/`UResult`/`UcallBuilder`/`warpper`）在 `ubi` crate
//! （镜像 sbi·ubi::ucall），本 crate 只留 `env` 域适配层、共享入口（`entry`）、
//! 用户堆（`heap`）与 task 模块（`task`，对齐内核 work::task）：域操作一段薄封装、
//! 零 asm；入口由各 bin 引用 `_start` 引导。

extern crate alloc;

/// 页大小（与内核 `memory::PAGE_SIZE` 对齐；堆分配按页对齐向上取整）。
pub const PAGE_SIZE: usize = 4096;

/// 共享入口：`_start` → `main` 引导 + panic 处理（bin 只需写 `main`）。
pub mod entry;

/// 用户堆：`heap_allocate`/`heap_deallocate` envcall 后端 + `#[global_allocator]`。
pub mod heap;

/// 用户 task：`spawn`/`closure`/`Join`（对齐内核 `work::task` 词汇）。
pub mod task;

/// 域适配层：每个域操作一段薄封装，只转发 `ubi::UcallBuilder`。
pub mod env {
    use core::time::Duration;
    use ubi::{UArgs, UResult, Ucall, UcallBuilder};

    use crate::PAGE_SIZE;

    /// 主动让出处理器（轮转；对齐内核 `scheduler::starve`）。
    pub fn starve() -> UResult<()> {
        let (_v0, _v1) = UcallBuilder::new(Ucall::Yield).call()?;
        Ok(())
    }

    /// 输出字符串（a0 = len，a1 = 缓冲指针）；ok = 该字符串已送出到控制台。
    /// 经 envcall Write 的字节直写——字符串字面量与 &[u8] 均可经 `as_bytes`/`as_ptr`。
    pub fn put(s: &str) -> UResult<()> {
        let args = UArgs {
            a0: s.len(),
            a1: s.as_ptr() as usize,
            ..UArgs::default()
        };
        let (_v0, _v1) = UcallBuilder::new(Ucall::Write).args(args).call()?;
        Ok(())
    }

    /// 退出当前任务；不返回（内核随后调度别的任务）。
    pub fn exit() -> ! {
        let _ = UcallBuilder::new(Ucall::Exit).call();
        unsafe { core::hint::unreachable_unchecked() }
    }

    /// 读定时器 tick 计数（诊断，非时间单位）。
    pub fn get_ticks() -> UResult<usize> {
        let (v0, _v1) = UcallBuilder::new(Ucall::GetTicks).call()?;
        Ok(v0)
    }

    /// 睡眠 `d`。a0 = d.as_millis()；亚毫秒截为 0 → 立即唤醒。
    pub fn sleep(d: Duration) -> UResult<()> {
        let args = UArgs {
            a0: d.as_millis() as usize,
            ..UArgs::default()
        };
        let (_v0, _v1) = UcallBuilder::new(Ucall::Sleep).args(args).call()?;
        Ok(())
    }

    /// 读单调时钟（uptime）：返回 (秒, 亚秒纳秒)。
    pub fn clock() -> UResult<(u64, u64)> {
        let (secs, nanos) = UcallBuilder::new(Ucall::ClockGetTime).call()?;
        Ok((secs as u64, nanos as u64))
    }

    /// 用户堆分配 `size` 字节（页对齐向上取整）：返回分配 VA，失败 UError（D1 负值）。
    pub fn heap_allocate(size: usize) -> UResult<usize> {
        let size = size.max(1).next_multiple_of(PAGE_SIZE);
        let args = UArgs {
            a0: size,
            ..UArgs::default()
        };
        let (v0, _v1) = UcallBuilder::new(Ucall::HeapAllocate).args(args).call()?;
        Ok(v0)
    }

    /// 用户堆释放 `(addr, size)`：与分配时同源页对齐，位图精确匹配；未分配/部分释放 → Err。
    pub fn heap_deallocate(addr: usize, size: usize) -> UResult<()> {
        let size = size.max(1).next_multiple_of(PAGE_SIZE);
        let args = UArgs {
            a0: addr,
            a1: size,
            ..UArgs::default()
        };
        let (_v0, _v1) = UcallBuilder::new(Ucall::HeapDeallocate).args(args).call()?;
        Ok(())
    }

    /// 建用户任务（Spawn envcall）：a0 = 入口 VA，a1 = arg；返回任务句柄（帧 PA）或 UError。
    pub fn spawn(entry: usize, arg: usize) -> UResult<usize> {
        let args = UArgs {
            a0: entry,
            a1: arg,
            ..UArgs::default()
        };
        let (v0, _v1) = UcallBuilder::new(Ucall::Spawn).args(args).call()?;
        Ok(v0)
    }
}
