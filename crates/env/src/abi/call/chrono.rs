//! call::chrono — **Chrono 域（class 4：时钟与到点）**：调用表（[`ChronoCall`]）。

use mold::Envcall;

/// 时钟调用（class 4；域 = runtime::chrono）。
#[derive(Envcall)]
#[call(class = 4)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChronoCall {
    /// 读取定时器 tick 计数（诊断，非时间单位）。
    #[infallible]
    #[ret(usize)]
    Ticks,
    /// 读取单调时钟（自启动基准）：**单字 `u64` 纳秒**。
    ///
    /// 这是本 ABI 的**绝对点**：与 [`RoomCall::ParkUntil`] 的 `at` **同基准、同单位**——
    /// 域读到的点可以（加一个周期之后）原样喂回去。单调不减；实际分辨率是机器
    /// timebase 的一跳（QEMU virt 10 MHz ⇒ **100 ns**，"纳秒"这个单位名比它细）。
    /// `u64` 纳秒覆盖约 584 年，不回绕。展示用的"秒 + 亚秒纳秒"由域自己拆
    /// （`/ 1e9`、`% 1e9`）——ABI 只给一个标量。
    #[infallible]
    #[ret(u64)]
    Clock,
}
