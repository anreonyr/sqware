//! 共享引擎的物理态：同一套 DDT／命令队列／故障队列在 Domain 之间只有一条升降路径。

use super::handshake::Handshake;

pub enum Engine {
    /// 队列都已停、DDTP 已回读为 Off。
    Off,
    /// 队列抬起只成功了一部分；只能继续降到 Off，不能从这里再抬。
    Raising,
    /// 队列活动且无错，DDTP 指向本引擎的目录表。
    On,
    /// 已写下降沿，正按同一条有界握手回读；这期间新发布只能等 Off。
    Lowering { handshake: Handshake },
}
