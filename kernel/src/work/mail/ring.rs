// ring — 共享内存邮路（mail 双通道之二）：数据/索引全在用户态共享内存（零拷贝），
// 内核侧不搬消息，只提供：
//   1. 连接状态 Live → Hang → Gone / Dead（共享帧内状态槽，双端 AMO 读、CAS 迁移）；
//   2. 同步：wait（空/满阻塞）与 wake（条件变更后唤醒）直用调度域原语
//      （Ucall::Wait / Ucall::Wake）——mail 不重造调度器。
//
// 首版（同空间）ring 连接 = 用户堆上一段共享区域（buffer + 原子读写索引 + 状态
// 槽）；内核侧无独立句柄，状态槽与索引都由用户侧原子访问。本文件固化为内核侧
// 语义契约与状态机（编译期核对依据），不持有运行状态。

/// ring 连接状态（共享状态槽的语义四态）。
///
/// 迁移律：open → Live；对端任务退出（Arc 递减钩子）→ Live → Hang；
/// 显式 shut → Live/Hang → Dead；Hang 且余信取空 → Gone（拉端钉连，
/// CAS 迁移——Hang → Gone 是唯一用户态可发起的迁移，闭「断开感知」窗口）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RingState {
    Live,
    Hang,
    Gone,
    Dead,
}

impl RingState {
    /// Hang → Gone：拉端在取空余信后钉连（CAS 前置断言由调用方持状态槽原子执行）。
    pub const fn hang_to_gone(&self) -> Option<RingState> {
        match self {
            RingState::Hang => Some(RingState::Gone),
            _ => None,
        }
    }

    /// 是否仍可消费（取信）：Live / Hang（余信未空）可取；Gone / Dead 断开。
    pub const fn pullable(&self) -> bool {
        matches!(self, RingState::Live | RingState::Hang)
    }
}
