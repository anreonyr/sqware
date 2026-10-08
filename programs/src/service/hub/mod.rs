//! 设备账那一族的实现侧（prog-hub 那一台域）。
//! 设备语义永远在持有那台设备的那一域（`programs/src/driver/…`）。
//! **核心与适配的边界**（与 operator / coalition 同一条）：核心不调用资源或执行原语、不出现
//! `View`、不叫盟册；"主人还答得出吗"是喂进去的一个闭包（mail::reserve 那一手在适配层）。

pub mod core;
pub mod serve;

mod fault;
pub use fault::Start;
