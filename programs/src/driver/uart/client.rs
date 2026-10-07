//! 客人那一面：树上那**两枚号** → **两端句柄**。
//!
//! 与 `driver/rtc/client.rs` 同位（客人不碰设备、不碰那一页的布局）：这一层只做一件事——
//! 按**两个名字**把每个方向那一枚号取回来，拼成 [`Console::rx`]（对面写、本端读）与
//! [`Console::tx`]（本端写、对面读）。
//!
//! **一枚门牌后面是一具完整的架**：那一枚号既是页（字节）也是铃（页上那一位"有事"），
//! 故 [`Reader::from_raw`] / [`Writer::from_raw`] 各只收**一枚号**（不像页＋铃那一版要两枚）。
//!
//! **两端的 `mode` 各归各的写端**：`rx` 那一侧满了丢哪一头是**对面**（驱动）的规矩，
//! 本端不必知道；`tx` 这一侧是本端自己写，故由调用方把 `tx_mode` 递进来——本层不替谁定策略。
//!
//! **次序**：客人由编排域排在 `uart` 之后起（`terminal/program.rs` 的 `after`），而对面是
//! **落完两枚砖才响 `Ready`** 的 ⇒ 取号这一趟没有"半落"的窗口；`Face::tile` 自己那条重试
//! 留给树那一侧的正常延迟。两个名字各自走一趟（译号 → 取那一枚）。
//!
//! **这一层也是量具那一档念的那一条**：`probe-rack-mount` / `probe-rack-guest` 用**同一个**
//! [`find`]（只是把 `road` 换成它们自己的试验场），故真机量到的就是客人这条路本身。

use env::{PieToken, Wait};
use protocol::common::path::{Path, PathBuf};
use ipc::rack::{Mode, Reader, Writer};
use protocol::driver;
use protocol::system::operator::Face;

use crate::driver::uart::core::frame::{self, Bytes};

/// 控制台那两面：读口（对面排空出来的一批）与写口（本端推的一条字）。
pub struct Console {
    /// **对面写、本端读**（那一边是驱动）。
    pub rx: Reader<Bytes>,
    /// **本端写、对面读**。
    pub tx: Writer<Bytes>,
}

/// 控制台那一块窗格在树上的路（`/svc/drv/uart`）——**一处说全**（名字住 [`frame`]，
/// 路头住 `protocol::driver::ROAD`；两边都不自己拼）。
pub fn road() -> Option<PathBuf> {
    driver::ROAD.try_join(frame::ME)
}

/// 找控制台：两个名字各走一趟（译号带重试 ＋ 取那一枚），拼成两端。
///
/// 任一格取不到（路没落 / 树答不上 / 那一枚映不进来）⇒ `None`——调用方按"这一档用不了"处置，
/// 不猜地址、也不退化成半条路。
pub fn find(tree: &Face, road: &Path, tx_mode: Mode, within: Wait) -> Option<Console> {
    let rx_page = token_of(tree, road, frame::RX, within)?;
    let tx_page = token_of(tree, road, frame::TX, within)?;
    let rx = Reader::<Bytes>::from_raw(rx_page)?;
    let tx = Writer::<Bytes>::from_raw(tx_page, tx_mode)?;
    Some(Console { rx, tx })
}

/// 两个名字各走的那一趟：**译号 → 取那一枚**（`Tile::token` 把那一枚经会话授进本端表）。
fn token_of(tree: &Face, road: &Path, leaf: &str, within: Wait) -> Option<PieToken> {
    let at = road.try_join(leaf)?;
    tree.tile(&at, within).ok()?.token(within).ok()
}
