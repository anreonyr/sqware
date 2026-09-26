//! program::setup — **实例化这一台要做的那几手**。
//!
//! 旧 `Program` 上三格静态装配需求（`tokens` / `channels` / `needs`）在这里并成**一张同类
//! 清单**：每一项 = "把它变成 Service 时多做的一手"。于是 `Program` 只剩一格装配需求。
//!
//! **它不负责 start**：`Channel` 在手（`Control::spawn` 之后）装泊位；`Need` 要等服务起来
//! 之后才递（`Control::wire`）。两件都由装配那一圈（`System::bring_up`）按次序落到
//! `Control` 那两手上——**没有 `apply` 这层转发**。

use plan::supply::Need;

/// 实例化一台要多做的一手。
#[derive(Clone, Copy)]
pub enum Setup {
    /// **资源**：这一台要一枚门闩（今天 = 收方那张需求单里的一条：PLIC / 设备树 / 门铃 /
    /// `ns16550a` / `goldfish-rtc` / `virtio,mmio`）。
    Need(Need),
    /// **通信**：这一台要开一条通道（今天只有 `records`）。放行前 `seat`，放行后按记号
    /// `claim`——它交回那一枚就是"它起来了"的证据，随后配给也从这条路上推过去。
    Channel(&'static str),
}
