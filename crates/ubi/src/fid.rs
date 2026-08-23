//! 环境调用号（ubi · fid），镜像 sbi::fid：调用号枚举 + usize 往返。
//! user 侧 `From<Ucall> for usize` 编码 a7，kernel 侧 `TryFrom<usize>` 解析 a7。

/// 环境调用号（a7）。变体名即 ABI 契约名，与 kernel `work::envcall` 分发表一一对应。
#[repr(usize)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ucall {
    /// 主动让出处理器（round-robin 轮转）。
    Yield = 0,
    /// 输出单字符（a0 = 字符码）。
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
    /// 建用户任务（a0 = 入口 VA，a1 = arg）：当前 team 建 U 任务，返回任务句柄或负错误码。
    Spawn = 8,
    /// 用户主动内核 panic（a0 = 任意关联码；不返回）——场景转储的显式触发。
    /// 取代「非法调用号撞 panic」的隐式路径（无消息、不可携带参数）；呼叫人即
    /// running 任务，trap 帧保留用户现场 → ubt/CSR 符号化完整可用。
    Panic = 9,
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
            _ => Err(()),
        }
    }
}
