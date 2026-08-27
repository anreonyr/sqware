//! 环境调用号（Ucall）枚举 + usize 往返。
//!
//! 命名与调度词族（conductor）及用户侧 API（`user::env`）同词：调度三词
//! Starve/Park/Reap 与词族一致；服务类与 `user::env` 函数同名
//! （Put/Ticks/Clock/Allocate/...）。

/// 环境调用号（a7）。
#[repr(usize)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ucall {
    /// 主动让出处理器（词族 starve）。
    Starve = 0,
    /// 写缓冲（a0 = len，a1 = 缓冲 VA）。
    Put = 1,
    /// 退出当前任务（不返回；词族 reap）。
    Reap = 2,
    /// 读取定时器 tick 计数（诊断，非时间单位）。
    Ticks = 3,
    /// 睡眠指定毫秒数（a0 = ms；词族 park）。
    Park = 4,
    /// 读取单调时钟（uptime）：a0 = 秒，a1 = 亚秒纳秒。
    Clock = 5,
    /// 用户堆分配（a0 = 字节数，页对齐向上取整）：返回分配 VA 或负错误码。
    Allocate = 6,
    /// 用户堆释放（a0 = VA，a1 = 字节数，页对齐）：0 或负错误码。
    Deallocate = 7,
    /// 建用户任务（a0 = 入口 VA，a1 = arg，a2 = 栈大小（0 = 缺省
    /// `TASK_STACK_SIZE`））：返回任务句柄或负错误码。
    Spawn = 8,
    /// 用户主动内核 panic（a0 = 任意关联码；不返回）。
    Panic = 9,
    /// 高位大段懒匿名映射（a0 = 字节数，页对齐；a2 = 期望 VA，0 = 窗口自选
    /// 高位）：返回映射 VA 或负错误码。
    Mmap = 10,
    /// 释放 mmap/声明区域（a0 = VA，a1 = 字节数，页对齐）：0 或负错误码。
    Munmap = 11,
    /// 修改映射区域保护标志（a0 = VA，a1 = 字节数，页对齐，a2 = 新权限
    /// PteFlags 位）：0 或负错误码。
    Mprotect = 12,
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
            0 => Ok(Self::Starve),
            1 => Ok(Self::Put),
            2 => Ok(Self::Reap),
            3 => Ok(Self::Ticks),
            4 => Ok(Self::Park),
            5 => Ok(Self::Clock),
            6 => Ok(Self::Allocate),
            7 => Ok(Self::Deallocate),
            8 => Ok(Self::Spawn),
            9 => Ok(Self::Panic),
            10 => Ok(Self::Mmap),
            11 => Ok(Self::Munmap),
            12 => Ok(Self::Mprotect),
            _ => Err(()),
        }
    }
}
