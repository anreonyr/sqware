//! canonical::adapt::console — **找控制台**：`/svc/drv/uart/{rx,tx}` 那两枚门牌。
//! 那一趟（名字 → 号 → 入口）与它那一圈"再问一次"的重试全在 [`Face::tile_of`] 里（"门牌由

use env::Wait;
use protocol::driver;
use protocol::service::operator::client::Face;
use runtime::env::mail::HolePie;

/// 要找的那位服务在树上的名字：**控制台**（`/svc/drv/uart`——名字用服务名；它是一块 Pane）。
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
    // 路是**驱动那一族的常量**（`/svc/drv`）接上控制台那一段（`uart`）——一处都不自己拼。
    let want = driver::ROAD.try_join(WANT)?;
    let (rx, tx) = (want.try_join(RX)?, want.try_join(TX)?);
    // 两条路各走一趟（译号带重试 ＋ 取那一枚），坐标只在这两句里。
    let rx_entry = tree.tile(&rx, wait).ok()?;
    let tx_entry = tree.tile(&tx, wait).ok()?;
    let rx = rx_entry.token(wait).ok()?;
    let tx = tx_entry.token(wait).ok()?;
    Some(Console {
        rx: HolePie::from_token(rx),
        tx: HolePie::from_token(tx),
    })
}
