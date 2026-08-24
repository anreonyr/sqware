//! 环境调用号（Ucall）枚举 + usize 往返。

/// 环境调用号（a7）。
#[repr(usize)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ucall {
    /// 主动让出处理器。
    Yield = 0,
    /// 写缓冲（a0 = len，a1 = 缓冲 VA）。
    Write = 1,
    /// 退出当前任务（不返回）。
    Exit = 2,
    /// 读取定时器 tick 计数（诊断，非时间单位）。
    GetTicks = 3,
    /// 睡眠指定毫秒数（a0 = ms）。
    Sleep = 4,
    /// 读取单调时钟（uptime）：a0 = 秒，a1 = 亚秒纳秒。
    ClockGetTime = 5,
    /// 用户堆分配（a0 = 字节数，页对齐向上取整）：返回分配 VA 或负错误码。
    HeapAllocate = 6,
    /// 用户堆释放（a0 = VA，a1 = 字节数，页对齐）：0 或负错误码。
    HeapDeallocate = 7,
    /// 建用户任务（a0 = 入口 VA，a1 = arg）：返回任务句柄或负错误码。
    Spawn = 8,
    /// 用户主动内核 panic（a0 = 任意关联码；不返回）。
    Panic = 9,
    /// 高位大段懒匿名映射（a0 = 字节数，页对齐）：返回映射 VA 或负错误码。
    Mmap = 10,
    /// 释放 mmap 区域（a0 = VA，a1 = 字节数，页对齐）：0 或负错误码。
    Munmap = 11,
}

impl From<Ucall> for usize {
    fn from(call: Ucall) -> Self {
        call as usize
    }
}

impl TryFrom<usize> for Ucall {
    type Error = ();

    fn try_from(number: usize) -> Result<Self, ()> {
        match number {
            0 => Ok(Self::Yield),
            1 => Ok(Self::Write),
            2 => Ok(Self::Exit),
            3 => Ok(Self::GetTicks),
            4 => Ok(Self::Sleep),
            5 => Ok(Self::ClockGetTime),
            6 => Ok(Self::HeapAllocate),
            7 => Ok(Self::HeapDeallocate),
            8 => Ok(Self::Spawn),
            9 => Ok(Self::Panic),
            10 => Ok(Self::Mmap),
            11 => Ok(Self::Munmap),
            _ => Err(()),
        }
    }
}
