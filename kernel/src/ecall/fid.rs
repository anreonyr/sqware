/// Base 扩展的 FID
#[repr(usize)]
#[derive(Clone, Copy)]
pub enum Base {
    /// 获取 SBI 规范版本
    /// 无参数
    GetSpecVersion = 0,

    /// 获取 SBI 实现 ID
    /// 无参数
    GetImplId = 1,

    /// 获取 SBI 实现版本
    /// 无参数
    GetImplVersion = 2,

    /// 探测扩展是否支持
    /// a0 = 要探测的扩展 EID
    ProbeExtension = 3,

    /// 获取 mvendorid
    /// 无参数
    GetMvendorid = 4,

    /// 获取 marchid
    /// 无参数
    GetMarchid = 5,

    /// 获取 mimpid
    /// 无参数
    GetMimpid = 6,
}

/// Timer 扩展的 FID
#[repr(usize)]
#[derive(Clone, Copy)]
pub enum Timer {
    /// 设置定时器
    /// a0-a1 = 64 位绝对时间值
    SetTimer = 0,
}

/// IPI 扩展的 FID
#[repr(usize)]
#[derive(Clone, Copy)]
pub enum Ipi {
    /// 发送 IPI
    /// a0 = hart_mask，a1 = hart_mask_base
    SendIpi = 0,
}

/// RFENCE 扩展的 FID
#[repr(usize)]
#[derive(Clone, Copy)]
pub enum Rfence {
    /// 远程执行 FENCE.I
    /// a0 = hart_mask，a1 = hart_mask_base
    RemoteFenceI = 0,

    /// 远程执行 SFENCE.VMA
    /// a0 = hart_mask，a1 = hart_mask_base，a2 = start_addr，a3 = size
    RemoteSfenceVma = 1,

    /// 远程执行 SFENCE.VMA（带 ASID）
    /// a0 = hart_mask，a1 = hart_mask_base，a2 = start_addr，a3 = size，a4 = asid
    RemoteSfenceVmaAsid = 2,

    /// 远程执行 HFENCE.GVMA（带 VMID）
    /// a0 = hart_mask，a1 = hart_mask_base，a2 = start_addr，a3 = size，a4 = vmid
    RemoteHfenceGvmaVmid = 3,

    /// 远程执行 HFENCE.GVMA
    /// a0 = hart_mask，a1 = hart_mask_base，a2 = start_addr，a3 = size
    RemoteHfenceGvma = 4,

    /// 远程执行 HFENCE.VVMA（带 ASID）
    /// a0 = hart_mask，a1 = hart_mask_base，a2 = start_addr，a3 = size，a4 = asid
    RemoteHfenceVvmaAsid = 5,

    /// 远程执行 HFENCE.VVMA
    /// a0 = hart_mask，a1 = hart_mask_base，a2 = start_addr，a3 = size
    RemoteHfenceVvma = 6,
}

/// HSM 扩展的 FID
#[repr(usize)]
#[derive(Clone, Copy)]
pub enum Hsm {
    /// 启动目标 Hart
    /// a0 = hartid，a1 = start_addr，a2 = opaque
    Start = 0,

    /// 停止当前 Hart
    /// 无参数
    Stop = 1,

    /// 获取 Hart 状态
    /// a0 = hartid
    GetStatus = 2,

    /// 挂起当前 Hart
    /// a0 = suspend_type，a1 = resume_addr，a2 = opaque
    Suspend = 3,
}

/// System Reset 扩展的 FID
#[repr(usize)]
#[derive(Clone, Copy)]
pub enum SystemReset {
    /// 系统复位或关机
    /// a0 = reset_type（32位），a1 = reset_reason（32位）
    SystemReset = 0,
}

/// PMU 扩展的 FID
#[repr(usize)]
#[derive(Clone, Copy)]
pub enum Pmu {
    /// 获取性能计数器数量
    /// 无参数
    NumCounters = 0,

    /// 获取计数器信息
    /// a0 = counter_idx
    CounterInfo = 1,

    /// 配置性能计数器
    /// a0 = counter_idx_base，a1 = counter_idx_mask，a2 = config_flags，a3 = event_idx，a4 = event_data
    ConfigCounter = 2,

    /// 启动性能计数器
    /// a0 = counter_idx_base，a1 = counter_idx_mask，a2 = start_flags，a3 = initial_value
    StartCounter = 3,

    /// 停止性能计数器
    /// a0 = counter_idx_base，a1 = counter_idx_mask，a2 = stop_flags
    StopCounter = 4,

    /// 读取性能计数器
    /// a0 = counter_idx
    CounterRead = 5,
}

/// Debug Console 扩展的 FID
#[repr(usize)]
#[derive(Clone, Copy)]
pub enum Dbcn {
    /// 向调试控制台写入数据
    /// a0 = 字节数，a1 = 内存地址低位，a2 = 内存地址高位
    ConsoleWrite = 0,

    /// 从调试控制台读取数据
    /// a0 = 字节数，a1 = 内存地址低位，a2 = 内存地址高位
    ConsoleRead = 1,
}

/// Legacy Console 扩展的 FID（legacy 扩展无 FID，用 0 占位；a0 传字符）
#[repr(usize)]
#[derive(Clone, Copy)]
pub enum LegacyConsole {
    /// 向控制台写一个字符
    /// a0 = 字符
    PutChar = 0,
}

/// System Suspend 扩展的 FID
#[repr(usize)]
#[derive(Clone, Copy)]
pub enum Suspend {
    /// 系统挂起
    /// a0 = suspend_type，a1 = resume_addr，a2 = opaque
    SystemSuspend = 0,
}

/// CPPC 扩展的 FID
#[repr(usize)]
#[derive(Clone, Copy)]
pub enum Cppc {
    /// 读取 CPPC 寄存器
    /// a0 = cppc_reg_id
    Read = 0,

    /// 写入 CPPC 寄存器
    /// a0 = cppc_reg_id，a1 = value
    Write = 1,

    /// 探测 CPPC 寄存器是否支持
    /// a0 = cppc_reg_id
    Probe = 2,
}

/// Nested Acceleration 扩展的 FID
#[repr(usize)]
#[derive(Clone, Copy)]
pub enum NaCl {
    /// 同步 CSR
    /// a0 = csr_num
    SyncCsr = 0,
}

macro_rules! impl_into {
    ($($ty:ty),*) => {
        $(
            impl From<$ty> for usize {
                fn from(val: $ty) -> Self {
                    val as usize
                }
            }
        )*
    };
}

impl_into!(
    Base,
    Timer,
    Ipi,
    Rfence,
    Hsm,
    SystemReset,
    Pmu,
    Dbcn,
    Suspend,
    Cppc,
    NaCl,
    LegacyConsole
);
