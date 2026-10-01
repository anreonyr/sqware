//! uart::program — **串口驱动**（`prog-uart`）的装配声明。
//!
//! **U 态**：持有 `serial@10000000`（banner 里那张 PMP 是 S/U (R,W)），把"收到字节就拉线"打开。

use crate::program::{Demand, Died, Ending, Identity, Program, Relation, Setup};

/// 它死在起手 / 常驻哪一步。
pub const E_UART: Died = 9;

// **照实记（"要的那一枚"搬回本域）**：这一份原先还开着本域那张需求单（`UART_WANTS`），而装配者
// 按同一张单替本域领设备。那一整条路退了（设备由本域自己走一趟设备账认领）⇒ **单子回了它自己的
// 域**（`driver/uart/desk.rs` 的 `ASK`），装配表上这一份只剩"它是谁、跟谁有边"。

pub static PROGRAM: Program = Program {
    identity: Identity {
        name: "uart",
        scenes: &["root", "product"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        deps: Some(&["operator", "hub"]),
        ending: Some(Ending::Resident),
        presence: true,
        ..Relation::DEFAULT
    },
    demand: Demand {
        died: E_UART,
        // **起手最后一步（落面）之后才交**：这一格就是「答得动」的凭据。
        setup: &[Setup::Ready(crate::program::READY)],
        ..Demand::DEFAULT
    },
};
