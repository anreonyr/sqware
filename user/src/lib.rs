#![no_std]
//! 用户态系统调用封装（U-mode → S-mode 环境调用）。

extern crate alloc;

/// 页大小。
pub const PAGE_SIZE: usize = 4096;

/// 共享入口：`_start` → `main` 引导 + panic 处理（bin 只需写 `main`）。
pub mod entry;

/// 用户堆：`allocate`/`deallocate` envcall 后端 + `#[global_allocator]`。
pub mod heap;

/// 用户 task：`spawn`/`closure`/`Join`。
pub mod task;

/// 域适配层：每个域操作一段薄封装，只转发 `ubi::UcallBuilder`。
pub mod env {
    use core::time::Duration;
    use ubi::{UArgs, UResult, Ucall, UcallBuilder};

    use crate::PAGE_SIZE;

    /// 主动让出处理器（词族 starve；轮转）。
    pub fn starve() -> UResult<()> {
        let (_v0, _v1) = UcallBuilder::new(Ucall::Starve).call()?;
        Ok(())
    }

    /// 输出字符串（a0 = len，a1 = 缓冲指针）。
    pub fn put(s: &str) -> UResult<()> {
        let args = UArgs {
            a0: s.len(),
            a1: s.as_ptr() as usize,
            ..UArgs::default()
        };
        let (_v0, _v1) = UcallBuilder::new(Ucall::Put).args(args).call()?;
        Ok(())
    }

    /// 退出当前任务（词族 reap）；不返回。
    pub fn exit() -> ! {
        let _ = UcallBuilder::new(Ucall::Reap).call();
        unsafe { core::hint::unreachable_unchecked() }
    }

    /// 读定时器 tick 计数（诊断，非时间单位）。
    pub fn ticks() -> UResult<usize> {
        let (v0, _v1) = UcallBuilder::new(Ucall::Ticks).call()?;
        Ok(v0)
    }

    /// 睡眠 `d`（词族 park）。a0 = d.as_millis()；亚毫秒截为 0 → 立即唤醒。
    pub fn sleep(d: Duration) -> UResult<()> {
        let args = UArgs {
            a0: d.as_millis() as usize,
            ..UArgs::default()
        };
        let (_v0, _v1) = UcallBuilder::new(Ucall::Park).args(args).call()?;
        Ok(())
    }

    /// 读单调时钟（uptime）：返回 (秒, 亚秒纳秒)。
    pub fn clock() -> UResult<(u64, u64)> {
        let (secs, nanos) = UcallBuilder::new(Ucall::Clock).call()?;
        Ok((secs as u64, nanos as u64))
    }

    /// 用户堆分配 `size` 字节（页对齐向上取整）：返回分配 VA，失败 UError（D1 负值）。
    pub fn allocate(size: usize) -> UResult<usize> {
        let size = size.max(1).next_multiple_of(PAGE_SIZE);
        let args = UArgs {
            a0: size,
            ..UArgs::default()
        };
        let (v0, _v1) = UcallBuilder::new(Ucall::Allocate).args(args).call()?;
        Ok(v0)
    }

    /// 用户堆释放 `(addr, size)`：与分配时同源页对齐；未分配/部分释放 → Err。
    pub fn deallocate(addr: usize, size: usize) -> UResult<()> {
        let size = size.max(1).next_multiple_of(PAGE_SIZE);
        let args = UArgs {
            a0: addr,
            a1: size,
            ..UArgs::default()
        };
        let (_v0, _v1) = UcallBuilder::new(Ucall::Deallocate).args(args).call()?;
        Ok(())
    }

    /// 高位大段懒匿名映射（mmap envcall）：a0 = 字节数（页对齐）；a2 = 期望 VA，
    /// 0 = 窗口自选高位。返回映射 VA（固定地址 / 高位）或 UError。触碰才分配
    /// 物理帧。
    pub fn mmap(size: usize) -> UResult<usize> {
        let size = size.max(1).next_multiple_of(PAGE_SIZE);
        let args = UArgs {
            a0: size,
            ..UArgs::default()
        };
        let (v0, _v1) = UcallBuilder::new(Ucall::Mmap).args(args).call()?;
        Ok(v0)
    }

    /// 固定地址声明式懒映射（mmap envcall 的 a2 模式）：在 `addr` 声明 `size`
    /// 字节匿名映射（页对齐、不得与既有映射/窗口重叠）；触碰才分配物理帧。
    pub fn mmap_at(addr: usize, size: usize) -> UResult<usize> {
        let size = size.max(1).next_multiple_of(PAGE_SIZE);
        let args = UArgs {
            a0: size,
            a2: addr,
            ..UArgs::default()
        };
        let (v0, _v1) = UcallBuilder::new(Ucall::Mmap).args(args).call()?;
        Ok(v0)
    }

    /// 释放 mmap/声明区域（munmap envcall）：a0 = VA，a1 = 字节数（与分配时
    /// 同源页对齐）；未命中 → Err。
    pub fn munmap(addr: usize, size: usize) -> UResult<()> {
        let size = size.max(1).next_multiple_of(PAGE_SIZE);
        let args = UArgs {
            a0: addr,
            a1: size,
            ..UArgs::default()
        };
        let (_v0, _v1) = UcallBuilder::new(Ucall::Munmap).args(args).call()?;
        Ok(())
    }

    /// 修改映射区域保护标志（Mprotect envcall）：a0 = VA，a1 = 字节数（页对齐），
    /// a2 = 新权限（PteFlags 位：V/R/W/X/U/G/A/D = bit 0..7）；未命中映射 → Err。
    pub fn mprotect(addr: usize, size: usize, flags: u64) -> UResult<()> {
        let size = size.max(1).next_multiple_of(PAGE_SIZE);
        let args = UArgs {
            a0: addr,
            a1: size,
            a2: flags as usize,
            ..UArgs::default()
        };
        let (_v0, _v1) = UcallBuilder::new(Ucall::Mprotect).args(args).call()?;
        Ok(())
    }

    /// 建用户任务（Spawn envcall）：a0 = 入口 VA，a1 = arg（a2 = 0 缺省栈）；
    /// 返回任务句柄或 UError。
    pub fn spawn(entry: usize, arg: usize) -> UResult<usize> {
        let args = UArgs {
            a0: entry,
            a1: arg,
            ..UArgs::default()
        };
        let (v0, _v1) = UcallBuilder::new(Ucall::Spawn).args(args).call()?;
        Ok(v0)
    }

    /// 建用户任务并指定栈大小（Spawn envcall 的 a2 模式）：`stack` 字节（0 =
    /// 缺省 `TASK_STACK_SIZE`；非 0 由内核页对齐）。返回任务句柄或 UError。
    pub fn spawn_with_stack(entry: usize, arg: usize, stack: usize) -> UResult<usize> {
        let args = UArgs {
            a0: entry,
            a1: arg,
            a2: stack,
            ..UArgs::default()
        };
        let (v0, _v1) = UcallBuilder::new(Ucall::Spawn).args(args).call()?;
        Ok(v0)
    }

    /// 用户主动内核 panic（a0 = 任意关联码；不返回）。
    pub fn panic_me(code: usize) -> ! {
        let args = UArgs {
            a0: code,
            ..UArgs::default()
        };
        let _ = UcallBuilder::new(Ucall::Panic).args(args).call();
        unsafe { core::hint::unreachable_unchecked() }
    }
}
