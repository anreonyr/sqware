//! program — **静态的节点**：我是谁（`name`）＋ 实例化我需要什么（`setup`）。
//!
//! 它**不是**运行时实例，也**没有方法**：没有状态、没有生命周期、没有协议角色，也没有
//! "从哪儿来"那一格——**每一条服务的身子的来路只有一种**：按 `name` 去清单里挑镜像，
//! 建域、产线程（`mint`）。iii 那套"同一份字节按 `args` 分派角色"（`Role` / `Source` /
//! 同域 `spawn_here`）连同它的绕路一起退了。
//!
//! ```text
//!   Program（静态声明）── Control::spawn/start/wire ──▶ Service（运行时：域 + 线程 + 会话）
//! ```

pub mod setup;

pub use setup::Setup;

/// 一条服务的**静态声明**。
#[derive(Clone, Copy)]
pub struct Program {
    /// 清单里的程序名 —— 也是本域给它起的服务名；身子按它去清单里挑。
    pub name: &'static str,
    /// 实例化它要做的那几手（资源 / 通信）。
    pub setup: &'static [Setup],
}
