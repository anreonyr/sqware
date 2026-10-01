//! ledger — **装机那几本账**：坐标（[`key`]）· 配对块（[`pair`]）· 启动参数（[`args`]）·
//! 清单（[`manifest`]）。
//! 它们是**两侧共读的字节布局**：打包那一侧（内核的 `build.rs` 与 `crates/image`）与读的那一侧
//! （域）用同一份定义，故不住任何一侧。

pub mod args;
pub mod key;
pub mod manifest;
pub mod pair;
