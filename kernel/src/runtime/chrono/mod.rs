//! chrono — 时间域（计时责任面）
//!
//! 组合（相互咬合，同属时间计量）：
//!   clock  — 时钟源：只答「现在几点了 / 过了多久」（time CSR 读数 + Duration 换算 + tick 基准）
//!   timer  — 计时触发：tock 日程（deadline 登记/取消/到期取走）+ 节拍计数；依赖 clock
//!
//! 分层：clock 不含任何 deadline 语义；timer 依赖 clock（now/换算/Instant），
//! clock 不反向依赖 timer。
pub mod clock;
pub mod timer;
