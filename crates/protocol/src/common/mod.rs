//! common — **协议的公共词表与机制**：面的生成（[`faces`]）· 树上的坐标（[`path`]）·
//! 命名树那几段的共同前缀（[`svc`]）· 记号的碰撞账（[`marks`]）。
//!
//! **协议树容纳那四套协议**（[`crate::system::control`] / `service::operator` / `service::principal`
//! / `service::coalition`）：判据是"谁住编排域"——它们的落地都是编排域里的线程（控制面那一枚线程、
//! 持树者 / 名册 / 盟册），而"要找服务得先有目录——那本目录就是树（`operator`）"这句写在
//! [`crate::common::path`] 的头注里。
//! ```text
//!   System Protocol = Control + Principal + Coalition + Operator     （四轴，平级）
//!     Control     系统里有什么 Service，它们处于什么生命状态
//!     Principal   一个 Task / 请求代表谁
//!     Coalition   哪些身份形成横向关系
//!     Operator    名字如何指向资源
//!   System 本身没有第五种对象：不是 kernel object、不是统一 Client、不负责通信机制。
//! ```
//! 它们与内核那一层彻底分开：
//! ```text
//!   内核 ABI（机制）     Unit                 Task 怎么创建、运行、结束
//!        │
//!   本协议（编排）       Service              系统由哪些 Service 组成、怎么运行
//!        │                                     （判定与账在本文这一侧）
//!   实现方               programs/src/        起一条、看/判、放下、收场
//! ```

pub mod faces;
pub mod marks;
pub mod path;
pub mod svc;
