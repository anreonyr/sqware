//! 钟到问一枚门铃，主人没了的格空出来。

use ::resource::raw::reserve;

use super::*;

/// 与线路由者那条探活同一手
pub(super) fn alive(sensor: PieToken) -> bool {
    ::resource::raw::alive(sensor)
        && matches!(reserve(sensor), Ok((_, _, mark)) if mark == hub::ALIVE_MARK)
}
