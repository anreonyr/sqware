//! boot — **内核对引导镜像那一域的两块账**：清单（装了哪些程序）与配对块（有哪些门闩）。
//! 这是**这台机器的事实**，不是协议：它读的是启动参数（`env::ledger::args`），答的还是
//! "谁被装进来了"这件事。
//! 内核只把这两区**只读借映**进**引导镜像那一域**，故读者是每一个引导镜像：
//! `system`，以及 harness 的 `again` / `rig` / `load` / `group`。

pub mod accounts;
pub mod catalog;

pub use accounts::Accounts;
pub use catalog::Catalog;
