//! 环境调用号（EnvCall）枚举 + 载荷 codec。
//!
//! 方案 3（typed payload）：每个原语是一个**带类型载荷的 variant**，字段类型是
//! envcall 语义句柄（`PieToken`/`TaskId`/`VirtAddr`）或 `Permission`/裸量。调用号
//! （a7）不再 `#[repr(usize)]`+手写 `as usize`，而由 `[derive(Envcall)]` 生成的
//! codec 现算：`(class << 32) | index`，`index` 是**声明顺序**判别号（重排即改
//! ABI，写进本文件注释即文档）。
//!
//! 返回类型（R3）：每个 variant 标 `#[ret(T)]`，derive 生成域 `*Ret` 枚举与
//! `call()`（负值即 `EnvError`，非负蒸馏为 Ret）。`call()` 绑定 envcall 汇编入口，
//! `slot/pack/unpack` 只依赖 `Wire`——sbi 未来可复用同一 derive。
//!
//! 分类与功能域一一对应（class=高 32 位）：Room=0, Task=1, Memory=2, IO=3,
//! Chrono=4, Mail=5, Control=6。**class 7 已删除**（原 `ServiceCall` 是入口策略
//! 而非原语：目录入口门闩改由父任务 `Accord` 下发，见 `docs/dispatch.md`）；
//! 7 号保留空号不复用。命名与调度词族（conductor）、`runtime::chrono` 域及用户侧
//! `task::env` 同词。
//!
//! 根除的两处 L3' 漏洞：`Permission`/`PteFlags` 的 unpack 走 `from_bits(...)`
//! `.ok_or(...)` 校验（见 [`Wire`](crate::wire::Wire)），非法位 → `Err`，不再
//! `from_bits_truncate` 静默截断。

use envmacros::Envcall;

use crate::wire::{PieToken, TaskId, TeamId, VirtAddr};

/// 调度词族调用（class 0；域 = work/room）。
#[derive(Envcall)]
#[call(class = 0)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RoomCall {
    /// 主动让出处理器（词族 starve）。
    #[ret(())]
    Starve,
    /// 睡眠指定毫秒数（词族 park）。
    #[ret(())]
    Park { millis: usize },
    /// 退出当前任务（不返回；词族 reap）。发散，无 Ret。
    #[ret(())]
    Reap,
    /// 事件等待（词族 wait）：key + 毫秒（usize::MAX = 永久）。
    #[ret(())]
    Wait { key: usize, millis: usize },
    /// 事件唤醒（词族 wake）：key；返回是否唤到人。
    #[ret(bool)]
    Wake { key: usize },
}

/// 执行单元调用（class 1）—— unit 域：team（域）与 task（线程）两个建单元操作。
#[derive(Envcall)]
#[call(class = 1)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnitCall {
    /// 建用户任务（在**当前** team 里产线程）：entry VA，arg，stack（0 = 缺省）。
    ///
    /// `arg` 是父任务给子任务的**第一个字**：写入新任务 a0，用户运行时在 `_start`
    /// 保存为引导参数（`task::env::task::arg`）。它是提示，不是通道——多值走权限表。
    #[ret(TaskId)]
    Spawn {
        entry: usize,
        arg: usize,
        stack: usize,
    },
    /// 取当前 task id（无参 → 0 = 无上下文）。
    #[ret(TaskId)]
    SelfId,
    /// 溯源：生我者的 task id（0 = 顶级域 / 父已亡）。
    #[ret(TaskId)]
    Sire,
    /// 我生的子域数量（heir 枚举的 first pass；0 = 无子域）。
    #[ret(usize)]
    HeirCount,
    /// 按索引取子域 TeamId（heir 枚举的 second pass；越界 → 0）。
    #[ret(TeamId)]
    Heir { index: usize },
    /// 在给定 team 下建线程（域内产 task）：team + entry + arg → TaskId。
    #[ret(TaskId)]
    SpawnTask {
        team: TeamId,
        entry: usize,
        arg: usize,
    },
}

/// 内存调用（class 2；trace 事件名 `MemoryEvent` 同词）。
#[derive(Envcall)]
#[call(class = 2)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MemoryCall {
    /// 用户堆分配（字节数，页对齐向上取整）。
    #[ret(VirtAddr)]
    Allocate { size: usize },
    /// 用户堆释放（VA，字节数，页对齐）。
    #[ret(())]
    Deallocate { addr: VirtAddr, size: usize },
    /// 高位大段懒匿名映射（字节数页对齐；at = 期望 VA，VirtAddr(0) = 窗口自选）。
    #[ret(VirtAddr)]
    Mmap { size: usize, at: VirtAddr },
    /// 释放 mmap/声明区域（VA，字节数，页对齐）。
    #[ret(())]
    Munmap { addr: VirtAddr, size: usize },
    /// 修改映射区域保护标志（VA，字节数页对齐，新权限 PteFlags 位）。
    #[ret(())]
    Mprotect { addr: VirtAddr, size: usize, flags: u64 },
}

/// IO 调用（class 3）。
#[derive(Envcall)]
#[call(class = 3)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IOCall {
    /// 写缓冲（len，缓冲 VA）。
    #[ret(())]
    Put { len: usize, buf: VirtAddr },
    /// 非阻塞读一字节；无输入 → -2 Busy。
    #[ret(u8)]
    Get,
}

/// 时钟调用（class 4；域 = runtime::chrono）。
#[derive(Envcall)]
#[call(class = 4)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChronoCall {
    /// 读取定时器 tick 计数（诊断，非时间单位）。
    #[ret(usize)]
    Ticks,
    /// 读取单调时钟（uptime）：(秒, 亚秒纳秒)。
    #[ret((u64, u64))]
    Clock,
}

/// 通信调用（class 5，mail）。用户句柄统一为 per-pie `token`（全局唯一）。
/// UnsealHole / UnsealPole 创建资源（返 token）；Push / Pull / Map / Unmap / Seal
/// / Accord / Narrow / Revoke / Collect / Release 走 pie 门闩。wait/wake 不进本类
/// ——mail 同步直用调度词族 `RoomCall::Wait/Wake`。
///
/// 两条轴不要混：`Unseal*` ↔ `Seal` 动的是**资源**；`Accord` ↔ `Revoke`（他人）
/// 与 `Collect` ↔ `Release`（自己）动的是**我手里那一份**。
#[derive(Envcall)]
#[call(class = 5)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MailCall {
    /// 解封 Hole（数据过内核管道）。
    #[ret(PieToken)]
    UnsealHole,
    /// 解封 Pole（页级安全内存；字节数页对齐）。
    #[ret(PieToken)]
    UnsealPole { bytes: usize },
    /// push msg：token + msg VA。
    #[ret(())]
    Push { token: PieToken, msg: VirtAddr },
    /// pull msg：token + 缓冲 VA。
    #[ret(())]
    Pull { token: PieToken, buf: VirtAddr },
    /// 借映 Pole 物理页进当前 task.space：token → VA。
    #[ret(VirtAddr)]
    Map { token: PieToken },
    /// 从当前 task.space 解除映射：token。
    #[ret(())]
    Unmap { token: PieToken },
    /// 封印资源（generic on Hole/Pole）：token。
    #[ret(())]
    Seal { token: PieToken },
    /// 转授子集给其他 Task：src_token + dst_id + subset → 新 pie 的 token（撤销句柄）。
    #[ret(PieToken)]
    Accord {
        src: PieToken,
        dst: TaskId,
        subset: crate::permission::Permission,
    },
    /// 收窄本 pie 权限（就地改写；Pole 同步降页表）：token + subset。
    #[ret(())]
    Narrow {
        token: PieToken,
        subset: crate::permission::Permission,
    },
    /// 收回授与他人的副本：dst_id + token。
    #[ret(())]
    Revoke { dst: TaskId, token: PieToken },
    /// 收拢：报出本任务权限表第 `index` 份（token + permission）。
    /// 越界 → `PieToken(0)`（无效哨兵，不报错）。
    #[ret((PieToken, crate::permission::Permission))]
    Collect { index: usize },
    /// 放下：自释本任务的一份门闩（Pole 同步 unmap）。表里无此 token → -1。
    #[ret(())]
    Release { token: PieToken },
}

/// 控制调用（class 6）。
#[derive(Envcall)]
#[call(class = 6)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ControlCall {
    /// 用户主动内核 panic（任意关联码；不返回）。发散，无 Ret。
    #[ret(())]
    Panic { code: usize },
    /// 用户自诊断：采样当前任务调用栈，把 pc 地址数组写进用户 buf，返回帧数。
    ///
    /// `buf` = 用户预分配的 `[usize; N]` 数组 VA；`frames` = 该数组最大容量。
    /// 内核经 `mail::copy_out` 写 `frames` 个 pc 到 buf；返回实际捕获帧数（`usize`），
    /// buf 非法（未映射/不可写）→ 负值（EnvError）。
    #[ret(usize)]
    Backtrace { buf: usize, frames: usize },
}

/// 环境调用号聚合（内核侧解码总入口）。
///
/// `from_wire(slot, regs)` 按 class（高 32 位）分派到各域的 `from_wire`，得到
/// `MailCall::Push { .. }` 等带载荷 variant，供 `dispatch` match。用户侧不再构造
/// 本枚举——直接 `MailCall::X.call()` 发起（R3+B）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EnvCall {
    Room(RoomCall),
    Unit(UnitCall),
    Memory(MemoryCall),
    IO(IOCall),
    Chrono(ChronoCall),
    Mail(MailCall),
    Control(ControlCall),
}

impl EnvCall {
    /// 由调用号 + 寄存器组解码回带载荷的聚合枚举。
    pub fn from_wire(slot: usize, regs: &[usize; 6]) -> Result<Self, crate::wire::Decode> {
        let class = slot >> 32;
        match class {
            0 => Ok(EnvCall::Room(RoomCall::from_wire(slot, regs)?)),
            1 => Ok(EnvCall::Unit(UnitCall::from_wire(slot, regs)?)),
            2 => Ok(EnvCall::Memory(MemoryCall::from_wire(slot, regs)?)),
            3 => Ok(EnvCall::IO(IOCall::from_wire(slot, regs)?)),
            4 => Ok(EnvCall::Chrono(ChronoCall::from_wire(slot, regs)?)),
            5 => Ok(EnvCall::Mail(MailCall::from_wire(slot, regs)?)),
            6 => Ok(EnvCall::Control(ControlCall::from_wire(slot, regs)?)),
            _ => Err(crate::wire::Decode::BadSlot),
        }
    }
}
