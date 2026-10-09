//! 不碰内核、不碰寄存器的那一半。
//! 它只吃**设备树那段字节**（适配层把借映进来的视图切好交给它），故可独立推理；
//! ——那一条线的账住本目录 `lines.rs`（协议只留形与码，见 router_client）。

pub mod lines;
pub mod sources;
