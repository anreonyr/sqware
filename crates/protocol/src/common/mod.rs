//! 面的生成（faces）· 树上的坐标（path）·
//! 命名树那几段的共同前缀（svc）· 记号的碰撞账（marks）。
//! **协议树容纳那四套协议**（crate::system::control / service::operator / service::principal
//! / service::coalition）：判据是"谁住编排域"——它们的落地都是编排域里的线程（控制面那一枚线程、
//! 持树者 / 名册 / 盟册），而"要找服务得先有目录——那本目录就是树（`operator`）"这句写在
//! crate::common::path 的头注里。
//! 它们与内核那一层彻底分开：

pub mod faces;
pub mod marks;
pub mod path;
pub mod svc;
