//! env 适配层 —— 函数封 `*Call::X{..}.call()` 的域 Ret，零业务逻辑。
//! 厚封装（Port / Bell / Pile）见 `crate::core`。
//!
//! **两条轴两个文件，其余一域一个**：
//!   - [`mail`] —— **通信面**：`MailCall`(5) ＋ `ToleCall`(9) 的转发，加**四枚句柄**
//!     （Hole / Nole / Pole / Tole）与 [`mail::Mate`]；
//!   - [`pie`] —— **权柄面**：`PieCall`(7) 的转发 ＋ [`pie::AnyPie`] 与它的四份实现。
//!
//! 5 与 7 是**两条正交的轴**（见 `env::fid` 文件头）；9 是接在 5 那一族上的**多路等待**
//! ——组的成员是孔的一个方向或一枚铃，故与 5 同住。其余按 class 一域一个：`room`(0) ·
//! `unit`(1) · `memory`(2) · `chrono`(4) · `control`(6) · `debug`(8)。
//!
//! 口径一句话：**句柄一处（`mail.rs`），权柄动词一处（`pie.rs`）**。`mail::X` 这个前缀
//! 今天仍能取到权柄轴的名字（`mail.rs` 把 `pie.rs` 的每一项 `pub use` 转出去，调用点
//! 一行没改），但**实现**只在 `pie.rs` 一处。
//!
//! **原 `io` 子模块（`IOCall::Put`/`Get` 的转发）已删**：设备不在内核手里之后，
//! "往哪里写、从哪里读"是持设备者的事——控制台服务持 UART 门闩自己读写，客户端
//! 走 `protocol::console` 的会话。故这一层不再有 IO 面。
//!
//! [`debug`] 与上一条不矛盾：它不碰设备，借的是**内核自己的** DBCN 出口——
//! "服务还没起来的引导期要能说一句话"是它唯一的存在理由。

pub mod chrono;
pub mod control;
pub mod debug;
pub mod mail;
pub mod memory;
pub mod pie;
pub mod room;
pub mod unit;
