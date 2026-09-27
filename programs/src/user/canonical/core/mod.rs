//! canonical::core — **纯功能**：不碰内核、不碰设备的那一半。
//!
//! ```text
//!   discipline.rs  行规程：canonical mode 的最小一份（ICRNL / ECHO / ECHOCTL / ERASE / KILL / EOF）
//! ```
//!
//! 本域的本职只有这一件：**把一条字节流按终端那一侧的规矩切成行**。碰内核的那几手（板、树、孔、
//! 轮转那一圈）住 `adapt/`（分界那一节见 `user/canonical/mod.rs`）。
//!
//! **不立 `frame.rs`、也不立会话核**：本域的对面是"一台终端"，不是另一份协议——没有报文要解、
//! 没有一问一答要记（与 `driver/uart/core/` 只有一条纪律同一条判据）。

pub mod discipline;
