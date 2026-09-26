//! uart::adapt::tree — **起手 4–5**：上树那一趟（三台共用）＋ 登记本域那一条线（两台共用）。
//!
//! 身子在 `programs::driver::tree::plate` 与 `programs::driver::register::occupy`；本文件只给
//! 这一台的事实：名字 `uart`、归属 [`Mine::Yes`]（"这枚读行的孔是我的"），以及"线 = 那一段区"。

use super::boot::Up;
use super::fail;
use env::Wait;
use programs::driver::register;
use programs::driver::tree::{self, Mine};
use protocol::debug;
use protocol::driver::line;

/// 本域挂在树上的名字：`/device/uart`（[`protocol::driver::DIR`] 之下的那一段，**服务名**）。
const ME: &str = "uart";

/// 办一趟登记的总上限（毫秒）。**必须有界**。
const MS: usize = 1000;

/// 4–5：上树那一趟，再把本域那一条线登记下来。
pub fn plate(up: &Up) -> Result<line::client::Line, fail::Fail> {
    // 上树那一趟（三台共用）：名字既是树上的那一段，也是读数前缀——`Mine::Yes` 说"这枚是我的"。
    tree::plate(
        ME,
        Mine::Yes,
        &up.link,
        up.talk,
        up.host,
        up.entry,
        Wait::AtMost(MS),
    );
    // 登记本域那条线：按名从树上找到线路由者（`/device/router`），报的是**发下来的那一段区**
    // ——"线 = 区的函数"那条权威在路由者那边解，本域从不说线号，也不自己造坐标。
    let held =
        register::occupy(&up.link, up.talk, up.key, Wait::AtMost(MS)).map_err(|_| fail::Fail::Line)?;
    debug!("uart: line occupied");
    Ok(held)
}
