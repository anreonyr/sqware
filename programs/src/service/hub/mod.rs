//! hub — **设备账那一族的实现侧**（`prog-hub` 那一台域）。
//! ```text
//!   core    纯：册（Entry / Cell / League）＋ 四格判定 ＋ 取窗        【已落】
//!   server  适配：收整机物料 → 立账 → 上树 → 立盟 → 落 /svc/hub 与 /dev → 三面待客 ＋ 探活
//!   main    入口：起手（`server::serve`）
//! ```
//! **它是服务，不是驱动**：读寄存器、排 FIFO、开除能都不在这里；它只办"这一台归谁"。
//! 设备语义永远在持有那台设备的那一域（`programs/src/driver/…`）。
//! **核心与适配的边界**（与 operator / coalition 同一条）：核心不出现 `runtime::`、不出现
//! `View`、不叫盟册；"主人还答得出吗"是喂进去的一个闭包（`mail::reserve` 那一手在适配层）。

pub mod core;
/// 那一枚线程（服务侧 / 持树侧）与它叫的那几手住这里。
pub mod serve;
