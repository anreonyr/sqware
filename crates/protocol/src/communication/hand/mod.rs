//! 一手：一枚孔上一条报。**一格缓冲的两半**住一份文件（`M::EMPTY` / `M::MAX` 是同一处）。
//!
//! 与 `communication::rack` 的关系：那边把"同样的字节过边界"换成一枚页上的 N 格（有界寄存），
//! 交接变寄存。两档**共用** `crate::wire::message::Message` 那四样，但**不共用类型**——
//! 失败域不同（这边的 `SendFail::Unbound` 对那边的 `SendFail::Full`），抽公共"发送端"只会
//! 造出一层没有名字的东西（见 `communication` 头注"本层不认识什么"）。

pub mod receiver;
pub mod sender;

pub use receiver::{Receiver, RecvFail};
pub use sender::{SendFail, Sender};
