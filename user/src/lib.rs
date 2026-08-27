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

/// 用户 mail：port 内核邮路阻塞封装。
pub mod mail;

/// 域适配层：每个域操作一段薄封装，只转发 `ubi::UcallBuilder`。
pub mod env {
    use core::time::Duration;
    use ubi::{
        ChronoCall, ControlCall, IOCall, MailCall, MemoryCall, RoomCall, TaskCall, UArgs, UResult,
        Ucall, UcallBuilder,
    };

    use crate::PAGE_SIZE;

    /// 主动让出处理器（词族 starve；轮转）。
    pub fn starve() -> UResult<()> {
        let (_v0, _v1) = UcallBuilder::new(Ucall::Room(RoomCall::Starve)).call()?;
        Ok(())
    }

    /// 输出字符串（a0 = len，a1 = 缓冲指针）。
    pub fn put(s: &str) -> UResult<()> {
        let args = UArgs {
            a0: s.len(),
            a1: s.as_ptr() as usize,
            ..UArgs::default()
        };
        let (_v0, _v1) = UcallBuilder::new(Ucall::IO(IOCall::Put)).args(args).call()?;
        Ok(())
    }

    /// 退出当前任务（词族 reap）；不返回。
    pub fn exit() -> ! {
        let _ = UcallBuilder::new(Ucall::Room(RoomCall::Reap)).call();
        unsafe { core::hint::unreachable_unchecked() }
    }

    /// 读定时器 tick 计数（诊断，非时间单位）。
    pub fn ticks() -> UResult<usize> {
        let (v0, _v1) = UcallBuilder::new(Ucall::Chrono(ChronoCall::Ticks)).call()?;
        Ok(v0)
    }

    /// 睡眠 `d`（词族 park）。a0 = d.as_millis()；亚毫秒截为 0 → 立即唤醒。
    pub fn sleep(d: Duration) -> UResult<()> {
        let args = UArgs {
            a0: d.as_millis() as usize,
            ..UArgs::default()
        };
        let (_v0, _v1) = UcallBuilder::new(Ucall::Room(RoomCall::Park)).args(args).call()?;
        Ok(())
    }

    /// 事件等待（词族 wait）：阻塞到 `wake(key)` 或超时；`ms == usize::MAX` = 永久。
    /// 唤醒后条件未必成立——调用方须重查共享状态（防漏唤醒由内核 pend 闩保证）。
    pub fn wait(key: usize, ms: usize) -> UResult<()> {
        let args = UArgs {
            a0: key,
            a1: ms,
            ..UArgs::default()
        };
        let (_v0, _v1) = UcallBuilder::new(Ucall::Room(RoomCall::Wait)).args(args).call()?;
        Ok(())
    }

    /// 事件唤醒（词族 wake）：给 `key` 投递信号；返回是否唤醒到等待者。
    pub fn wake(key: usize) -> UResult<usize> {
        let args = UArgs {
            a0: key,
            ..UArgs::default()
        };
        let (v0, _v1) = UcallBuilder::new(Ucall::Room(RoomCall::Wake)).args(args).call()?;
        Ok(v0)
    }

    /// 读单调时钟（uptime）：返回 (秒, 亚秒纳秒)。
    pub fn clock() -> UResult<(u64, u64)> {
        let (secs, nanos) = UcallBuilder::new(Ucall::Chrono(ChronoCall::Clock)).call()?;
        Ok((secs as u64, nanos as u64))
    }

    /// 用户堆分配 `size` 字节（页对齐向上取整）：返回分配 VA，失败 UError（D1 负值）。
    pub fn allocate(size: usize) -> UResult<usize> {
        let size = size.max(1).next_multiple_of(PAGE_SIZE);
        let args = UArgs {
            a0: size,
            ..UArgs::default()
        };
        let (v0, _v1) = UcallBuilder::new(Ucall::Memory(MemoryCall::Allocate)).args(args).call()?;
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
        let (_v0, _v1) = UcallBuilder::new(Ucall::Memory(MemoryCall::Deallocate)).args(args).call()?;
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
        let (v0, _v1) = UcallBuilder::new(Ucall::Memory(MemoryCall::Mmap)).args(args).call()?;
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
        let (v0, _v1) = UcallBuilder::new(Ucall::Memory(MemoryCall::Mmap)).args(args).call()?;
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
        let (_v0, _v1) = UcallBuilder::new(Ucall::Memory(MemoryCall::Munmap)).args(args).call()?;
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
        let (_v0, _v1) = UcallBuilder::new(Ucall::Memory(MemoryCall::Mprotect)).args(args).call()?;
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
        let (v0, _v1) = UcallBuilder::new(Ucall::Task(TaskCall::Spawn)).args(args).call()?;
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
        let (v0, _v1) = UcallBuilder::new(Ucall::Task(TaskCall::Spawn)).args(args).call()?;
        Ok(v0)
    }

    /// 用户主动内核 panic（a0 = 任意关联码；不返回）。
    pub fn panic_me(code: usize) -> ! {
        let args = UArgs {
            a0: code,
            ..UArgs::default()
        };
        let _ = UcallBuilder::new(Ucall::Control(ControlCall::Panic)).args(args).call();
        unsafe { core::hint::unreachable_unchecked() }
    }

    /// 建 port 通道（mail 内核邮路）：返回 (句柄, 条件键)。
    pub fn port_open() -> UResult<(usize, usize)> {
        let (h, k) = UcallBuilder::new(Ucall::Mail(MailCall::PortOpen)).call()?;
        Ok((h, k))
    }

    /// 终止 port 通道：0 或 UError。
    pub fn port_shut(handle: usize) -> UResult<()> {
        let args = UArgs {
            a0: handle,
            ..UArgs::default()
        };
        UcallBuilder::new(Ucall::Mail(MailCall::PortShut))
            .args(args)
            .call()?;
        Ok(())
    }

    /// 投递尝试（非阻塞）：成功 0；`UError` 负码 -2 = 槽满（Busy）、-1 = Dead。
    pub fn port_try_push(handle: usize, msg: *const u8) -> UResult<()> {
        let args = UArgs {
            a0: handle,
            a1: msg as usize,
            ..UArgs::default()
        };
        UcallBuilder::new(Ucall::Mail(MailCall::PortPush))
            .args(args)
            .call()?;
        Ok(())
    }

    /// 收取尝试（非阻塞）：成功写入 `buf`；`UError` 负码 -2 = 槽空（Busy）、-1 = Dead。
    pub fn port_try_pull(handle: usize, buf: *mut u8) -> UResult<()> {
        let args = UArgs {
            a0: handle,
            a1: buf as usize,
            ..UArgs::default()
        };
        UcallBuilder::new(Ucall::Mail(MailCall::PortPull))
            .args(args)
            .call()?;
        Ok(())
    }
}
