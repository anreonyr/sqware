//! 全协议一个号。
//! 生成的 `fail_to_code` / `code_to_fail` 就是拿它当"没失败"那一格：
//! `None ⇒ OK`、`OK ⇒ None`。
pub const OK: u8 = 0;
