//! program::system — **编排域**（`prog-system`）自己的装配声明。
//! 本文件**不在 `system` 那棵模块树里**（那棵树拖着 runtime / protocol，`crates/image` 进不去）
//! ——它由 `programs/src/unit/catalog.rs` 的 `#[path]` 拉进注册表，路径是 `crate::unit::system`。
//! 它是**这一景的引导镜像**（**不在装配单上**：`after: None`）：起手自己读 boot 的两块账，此后按
//! 各台声明里的 `after` **算出来的次序**依次起服务（`unit::order_scene`）。

use crate::unit::{Demand, Died, Identity, Relation, UnitFile};
use env::ProgramKind;

/// 起手第一步没成：两块账读不出来。
pub const E_BOOT: Died = 1;

pub static PROGRAM: UnitFile = UnitFile {
    identity: Identity {
        name: "system",
        space: ProgramKind::Supervisor,
        wanted_by: &["root", "product"],
        // **这两景的引导镜像**（并域那一刀：这个位子原是那个叫 `root` 的域的）。
        entry: &["root", "product"],
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};
