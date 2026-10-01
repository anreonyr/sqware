//! hub::serve::sweep — **探活那一格**：钟到问一枚门铃，主人没了的格空出来。

use super::*;

/// **"这一枚的主人还在吗"**——只有一条判据：那一枚还在本域表里。
/// 主人一死，内核把它的派生边（交给本域的那一份副本）一起摘掉 ⇒ `reserve` 答不出就是"没了"。
/// 与线路由者那条探活同一手。
pub(super) fn alive(sensor: PieToken) -> bool {
    mail::reserve(sensor).is_ok()
}
