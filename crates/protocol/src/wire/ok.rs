//! ok — **答话那一格的"没失败"**（`0`）：全协议**一个号**。
//!
//! 定义在这里；各族只把它转出来（`pub use crate::wire::OK;`）——`#[derive(WireCodes)]`
//! 生成的 `fail_to_code` / `code_to_fail` 就是拿它当"没失败"那一格：
//! `None ⇒ OK`、`OK ⇒ None`。
pub const OK: u8 = 0;
