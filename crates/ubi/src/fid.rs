//! 环境调用号（Ucall）枚举 + usize 往返。
//!
//! 槽号编码 = `(class << 32) | index`——高半 usize 是功能分类（外层 [`Ucall`]
//! 成员），低半是类内序号（子枚举判别式）。分类与功能域一一对应，**分类进
//! 类型层**（载荷形式 Type(Name)，与 trace 的 `EventKind::Room(RoomEvent)` 聚合
//! 同构；trace 事件名同步同词）：
//!
//!   Room    调度词族   Starve Park Reap Wait Wake
//!   Task    任务       Spawn
//!   Memory  内存       Allocate Deallocate Mmap Munmap Mprotect
//!   IO      IO         Put Get
//!   Chrono  时钟       Ticks Clock
//!   Mail    通信       PortOpen PortShut PortPush PortPull DockOpen DockShut
//!                     DockJoin DockClone DockDrop RingOpen RingClose RingJoin
//!   Control 控制       Panic
//!
//! 命名与调度词族（conductor）、`runtime::chrono` 域及用户侧 API
//! （`user::env`）同词；`Room`/`Memory` 与 trace 的 `RoomEvent`/`MemoryEvent`
//! 同词。

/// 调度词族调用（class 0；域 = work/room）。
#[repr(usize)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RoomCall {
    /// 主动让出处理器（词族 starve）。
    Starve = 0,
    /// 睡眠指定毫秒数（a0 = ms；词族 park）。
    Park = 1,
    /// 退出当前任务（不返回；词族 reap）。
    Reap = 2,
    /// 事件等待（词族 wait）：a0 = key，a1 = 毫秒（usize::MAX = 永久）。
    Wait = 3,
    /// 事件唤醒（词族 wake）：a0 = key；返回是否唤到人。
    Wake = 4,
}

/// 任务调用（class 1）。
#[repr(usize)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskCall {
    /// 建用户任务（a0 = 入口 VA，a1 = arg，a2 = 栈大小（0 = 缺省
    /// `TASK_STACK_SIZE`））：返回任务句柄或负错误码。
    Spawn = 0,
}

/// 内存调用（class 2；trace 事件名 `MemoryEvent` 同词）。
#[repr(usize)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MemoryCall {
    /// 用户堆分配（a0 = 字节数，页对齐向上取整）：返回分配 VA 或负错误码。
    Allocate = 0,
    /// 用户堆释放（a0 = VA，a1 = 字节数，页对齐）：0 或负错误码。
    Deallocate = 1,
    /// 高位大段懒匿名映射（a0 = 字节数，页对齐；a2 = 期望 VA，0 = 窗口自选
    /// 高位）：返回映射 VA 或负错误码。
    Mmap = 2,
    /// 释放 mmap/声明区域（a0 = VA，a1 = 字节数，页对齐）：0 或负错误码。
    Munmap = 3,
    /// 修改映射区域保护标志（a0 = VA，a1 = 字节数，页对齐，a2 = 新权限
    /// PteFlags 位）：0 或负错误码。
    Mprotect = 4,
}

/// IO 调用（class 3）。
#[repr(usize)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IOCall {
    /// 写缓冲（a0 = len，a1 = 缓冲 VA）。
    Put = 0,
    /// 非阻塞读一字节（a0 = 字节；无输入 → -2 Busy）。
    Get = 1,
}

/// 时钟调用（class 4；域 = runtime::chrono）。
#[repr(usize)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChronoCall {
    /// 读取定时器 tick 计数（诊断，非时间单位）。
    Ticks = 0,
    /// 读取单调时钟（uptime）：a0 = 秒，a1 = 亚秒纳秒。
    Clock = 1,
}

/// 通信调用（class 5，mail）。v1 七个：OpenHole / OpenPole 创建资源（返 pie_idx），
/// Push / Pull / Map / Unmap / Shut 走 pie 门闩。wait/wake 不进本类——
/// mail 同步直用调度词族 `Ucall::Room::Wait/Wake`。
#[repr(usize)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MailCall {
    /// 创建 Hole（数据过内核管道）：无参 → a0 = pie_idx。
    OpenHole = 0,
    /// 创建 Pole（页级安全内存）：a0 = 字节数（页对齐）→ a0 = pie_idx。
    OpenPole = 1,
    /// push msg：a0 = pie_idx，a1 = msg VA。
    Push = 2,
    /// pull msg：a0 = pie_idx，a1 = 缓冲 VA。
    Pull = 3,
    /// 借映 Pole 物理页进当前 task.space：a0 = pie_idx → a0 = VA。
    Map = 4,
    /// 从当前 task.space 解除映射：a0 = pie_idx。
    Unmap = 5,
    /// 终止资源（generic on Hole/Pole）：a0 = pie_idx。
    Shut = 6,
}

/// 控制调用（class 6）。
#[repr(usize)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ControlCall {
    /// 用户主动内核 panic（a0 = 任意关联码；不返回）。
    Panic = 0,
}

/// 环境调用号（a7）：外层按功能分类、载荷为类内调用（Type(Name) 聚合）。
#[repr(usize)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ucall {
    /// 调度词族（work/room）。
    Room(RoomCall),
    /// 任务。
    Task(TaskCall),
    /// 内存。
    Memory(MemoryCall),
    /// IO。
    IO(IOCall),
    /// 时钟（runtime/chrono）。
    Chrono(ChronoCall),
    /// 通信（mail）。
    Mail(MailCall),
    /// 控制。
    Control(ControlCall),
}

impl From<Ucall> for usize {
    fn from(call: Ucall) -> Self {
        match call {
            Ucall::Room(r) => r as usize,
            Ucall::Task(t) => (1usize << 32) | (t as usize),
            Ucall::Memory(m) => (2usize << 32) | (m as usize),
            Ucall::IO(i) => (3usize << 32) | (i as usize),
            Ucall::Chrono(c) => (4usize << 32) | (c as usize),
            Ucall::Mail(m) => (5usize << 32) | (m as usize),
            Ucall::Control(c) => (6usize << 32) | (c as usize),
        }
    }
}

impl TryFrom<usize> for Ucall {
    type Error = ();

    fn try_from(slot: usize) -> Result<Self, ()> {
        let class = slot >> 32;
        let index = slot & 0xFFFF_FFFF;
        match class {
            0 => Ok(Ucall::Room(RoomCall::try_from(index)?)),
            1 => Ok(Ucall::Task(TaskCall::try_from(index)?)),
            2 => Ok(Ucall::Memory(MemoryCall::try_from(index)?)),
            3 => Ok(Ucall::IO(IOCall::try_from(index)?)),
            4 => Ok(Ucall::Chrono(ChronoCall::try_from(index)?)),
            5 => Ok(Ucall::Mail(MailCall::try_from(index)?)),
            6 => Ok(Ucall::Control(ControlCall::try_from(index)?)),
            _ => Err(()),
        }
    }
}

macro_rules! index_from {
    ($($e:ident { $($n:ident = $v:literal),+ $(,)? })+) => {
        $(
            impl TryFrom<usize> for $e {
                type Error = ();
                fn try_from(index: usize) -> Result<Self, ()> {
                    match index { $($v => Ok(Self::$n),)+ _ => Err(()) }
                }
            }
        )+
    };
}

index_from! {
    RoomCall { Starve = 0, Park = 1, Reap = 2, Wait = 3, Wake = 4 }
    TaskCall { Spawn = 0 }
    MemoryCall { Allocate = 0, Deallocate = 1, Mmap = 2, Munmap = 3, Mprotect = 4 }
    IOCall { Put = 0, Get = 1 }
    ChronoCall { Ticks = 0, Clock = 1 }
    MailCall {
        OpenHole = 0, OpenPole = 1, Push = 2, Pull = 3,
        Map = 4, Unmap = 5, Shut = 6
    }
    ControlCall { Panic = 0 }
}
