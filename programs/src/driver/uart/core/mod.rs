//! 不碰内核、不碰设备的那一半。
//! `driver/uart/mod.rs`）——故这一层只有"这一批能不能交"这一条纪律：没有 `frame.rs`、
//! 也没有会话核。碰内核与设备的那几手住 `adapt/`，设备面住 `dev/uart.rs`。

pub mod batch;
