//! canonical::adapt::console — **找控制台**：`/device/uart/{rx,tx}` 那两枚门牌。
//!
//! 那一趟（名字 → 号 → 入口）与它那一圈"再问一次"的重试全在 [`Face::tile_of`] 里（"门牌由
//! 别的域落下，本域可能比它先起"）；本文件只剩"往哪找"这一格：目录是驱动族那一段，再下一段是
//! **服务名**（[`WANT`]），最后一段是**两面**（[`RX`] 读 / [`TX`] 写）。
//!
//! **缺任一枚都算没找到**：读得到、写不出去的那一台没有意义——本域正是靠写口把回显送回去。
//!
//! **`wait` 由调用方给**：等多久是**本域自己的期限**（[`super::MS`]），本文件不替它定。
//!
//! **本手只认一面 [`Face`]**：调用方（`main.rs`）**正握着那条会话的所有权**、此后不再要它
//! ⇒ 按"已持 `Session` 则用 `Face`"的规则，`main` 把它包成一面交进来（那条线怎么走不再出现在
//! 签名里）。"名字 → 号 → 入口"那一趟落在 [`Face::tile`] ＋ [`Tile::token`] 上。

use env::{Name, Wait};
use protocol::driver::DIR;
use protocol::system::operator::client::Face;
use runtime::env::mail::HolePie;

/// 要找的那位服务在树上的名字：**控制台**（`/device/uart`——名字用服务名；它是一块 Pane）。
const WANT: &str = "uart";

/// 那块 Pane 下的两枚门牌：[`RX`] = **读口**（控制台排空出来的一批，本域取），
/// [`TX`] = **写口**（本域推"一条完整的字"，控制台写进设备）。
const RX: &str = "rx";
const TX: &str = "tx";

/// 控制台那两面。
pub struct Console {
    pub rx: HolePie,
    pub tx: HolePie,
}

/// 找控制台（有界）：任一面的门牌找不到 ⇒ `None`（`main` 据此报
/// [`E_NO_CONSOLE`](super::E_NO_CONSOLE)）。
pub fn find(tree: &Face, wait: Wait) -> Option<Console> {
    let (Ok(dir), Ok(want), Ok(rx), Ok(tx)) = (
        Name::new(DIR),
        Name::new(WANT),
        Name::new(RX),
        Name::new(TX),
    ) else {
        return None;
    };
    // 两条路各走一趟（译号带重试 ＋ 取那一枚），坐标只在这两句里。
    let rx_entry = tree.tile(&[dir, want, rx], wait).ok()?;
    let tx_entry = tree.tile(&[dir, want, tx], wait).ok()?;
    let rx = rx_entry.token(wait).ok()?;
    let tx = tx_entry.token(wait).ok()?;
    Some(Console {
        rx: HolePie::from_token(rx),
        tx: HolePie::from_token(tx),
    })
}
