//! 引导线程的启动参数——boot 交给 root 的**入口账**。
//!
//! `Spawn` 的 `args` 是一串裸字（内核写到新任务栈顶，子方经 `a0`/`a1` 取回），故这段
//! 布局是 boot 与 root 之间的契约。定义放这里、两侧引同一份——与 [`pair`](crate::pair)、
//! [`manifest`](crate::manifest) 同一条理由：**跨域的字节布局不留第二份账**。
//!
//! ```text
//! [0] 清单区 VA   [1] 清单字节数   [2] 配对块 VA   [3] 配对块条数
//! ```
//!
//! 两个区都是 boot **只读借映**进 root 空间的（`kernel/src/boot.rs::spawn_root`）：VA 由
//! boot 在 root 的用户段里登记，长度即各自 region 的字节数，故 root 只读、不必自查。

/// 清单区 VA。
pub const VIEW: usize = 0;
/// 清单区的字节数。
pub const VIEW_LEN: usize = 1;
/// 配对块 VA。
pub const PAIRS: usize = 2;
/// 配对块的条数。
pub const COUNT: usize = 3;
/// 本布局的字数（`Spawn` 的 `count` 传它）。
pub const LEN: usize = 4;
