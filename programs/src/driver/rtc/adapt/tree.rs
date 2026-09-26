//! rtc::adapt::tree — **起手 4**：上树那一趟（三台共用）＋ 登记本域那一条线（两台共用）。
//!
//! 身子在 `programs::driver::tree::plate`（三台逐字同构的那一趟）与
//! `programs::driver::register::occupy`（两台同构的那一趟）；本文件只给这一台的事实：
//! 名字 `rtc`、归属 [`Mine::No`]（门牌公开可查，谁都能查、谁都能用），以及"线 = 发下来的那一段区"。

use super::boot::Up;
use super::fail;
use env::Wait;
use programs::driver::register;
use programs::driver::tree::{self, Mine};
use protocol::debug;
use protocol::driver::line;

/// 本域挂在树上的名字：`/device/rtc`（[`protocol::driver::DIR`] 之下的那一段，**服务名**）。
const ME: &str = "rtc";

/// 办一趟登记的总上限（毫秒）。**必须有界**。
const MS: usize = 1000;

/// 4：上树那一趟，再把本域那一条线登记下来。
///
/// 坐标**随配给记录发下来**（内核按 `reg` 段造的门闩；本域既不写死名字、也不写死地址）——
/// 取它这一步的**次序照旧**：在那一趟之后（失败路径上"先报树那一行、再死"与原先一致）。
pub fn plate(up: &Up) -> Result<line::client::Line, fail::Fail> {
    tree::plate(
        ME,
        Mine::No,
        &up.link,
        up.talk,
        up.host,
        up.entry,
        Wait::AtMost(MS),
    );
    let key = up.pie.key().ok_or(fail::Fail::Line)?;
    let held = register::occupy(&up.link, up.talk, key, Wait::AtMost(MS))
        .map_err(|_| fail::Fail::Line)?;
    debug!("rtc: line occupied");
    Ok(held)
}
