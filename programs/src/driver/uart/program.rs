//! uart::program — **串口驱动**（`prog-uart`）的装配声明。
//!
//! **U 态**：持有 `serial@10000000`（banner 里那张 PMP 是 S/U (R,W)），把"收到字节就拉线"打开。

use crate::unit::{Demand, Died, Ending, Identity, UnitFile, Relation, Setup};

/// 它死在起手 / 常驻哪一步。
pub const E_UART: Died = 9;

// **照实记（"要的那一枚"搬回本域）**：这一份原先还开着本域那张需求单（`UART_WANTS`），而装配者
// 按同一张单替本域领设备。那一整条路退了（设备由本域自己走一趟设备账认领）⇒ **单子回了它自己的
// 域**（`driver/uart/desk.rs` 的 `ASK`），装配表上这一份只剩"它是谁、跟谁有边"。

pub static PROGRAM: UnitFile = UnitFile {
    identity: Identity {
        name: "uart",
        wanted_by: &["root", "product"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        // **`router` 这一条边是补上的**（照实记：与 `rtc` 同一处成因）：本台起手也要问路由者
        // （`uart/desk.rs` 的 `ctx.line(...)`：查 `/svc/drv/router` 那一格、请它占线），
        // 而 `after` 里从前只有 `operator` / `hub`——那条依赖靠的是**位次**。
        // **那条红与这条边分开记**（A/B 两面都出得来、n=3 分不开，见 `rtc/program.rs` 的照实记）。
        after: Some(&["operator", "hub", "router"]),
        restart: Some(Ending::Resident),
        ..Relation::DEFAULT
    },
    demand: Demand {
        died: E_UART,
        // **起手最后一步（落面）之后才交**：这一格就是「答得动」的凭据。
        supply: &[Setup::Ready],
        ..Demand::DEFAULT
    },
};
