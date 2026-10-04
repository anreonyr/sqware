//! 一条中断线从"归了某个域"到"这一条处理完了"这一整段。
//! 它是 crate::driver 里**唯一那一件协议**（本层另一格是名字 super::DIR）。

pub mod frame;

pub use frame::Fail;

pub mod client;
pub use client::Line;

// **客侧留在本侧**（`client`）：它自己铸孔、自己 `claim`，碰内核；形与据住同层（`frame`）。
