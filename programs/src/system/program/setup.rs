//! program::setup — **实例化这一台要做的那几手**。
//!
//! 旧 `Program` 上三格静态装配需求（`tokens` / `channels` / `needs`）在这里并成**一张同类
//! 清单**：每一项 = "把它变成 Service 时多做的一手"。于是 `Program` 只剩一格装配需求，
//! `Program::assemble` 也只剩一个循环——机器不必再知道"通道"和"资源"是两种格子。
//!
//! **它不负责 start**：[`Setup::apply`] 只做放行前就该做的那几手；门闩单要等服务起来之后
//! 才递（`Control::wire`），故 [`Setup::Need`] 在装配期**什么都不做**，只是把"要什么"写下来。

use plan::supply::Need;

use crate::system::control::{Control, Error, Service};

/// 实例化一台要多做的一手。
#[derive(Clone, Copy)]
pub enum Setup {
    /// **资源**：这一台要一枚门闩（今天 = 收方那张需求单里的一条：PLIC / 设备树 / 门铃 /
    /// `ns16550a` / `goldfish-rtc` / `virtio,mmio`）。
    ///
    /// 装配期不动手：坐标要读树才定得下来，而递单要等它起来（`Control::wire`）。
    Need(Need),
    /// **通信**：这一台要开一条通道（今天只有 `records`）。
    ///
    /// 放行前 `seat`（在它的码头上装一条泊位），放行后按记号 `claim`——**它交回那一枚就是
    /// "它起来了"的证据**，随后配给也从这条路上推过去。
    Channel(&'static str),
}

impl Setup {
    /// 把这一手落到 `service` 上。失败 ⇒ 这一台没装配成（号由 System 那一侧折）。
    pub fn apply(&self, control: &mut Control, service: &mut Service) -> Result<(), Error> {
        match self {
            // 资源：**记下"起来之后要领这一枚"**（`Control::wire` 按这张单的次序递出去）。
            Setup::Need(need) => service.need(*need),
            // 通信：在它的码头上装一条泊位（放行前），记号留给放行后逐条认领。
            Setup::Channel(ch) => control.connect(service, ch),
        }
    }
}
