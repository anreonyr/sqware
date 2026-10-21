//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! `../unit/catalog.rs` 的 `PROGRAMS`）。

use crate::unit::{Ending, UnitFile};

/// **"退场即撤订"那一对的前半**：订一条路、停一下、退场（见 `main.rs` 头注）。
///
/// 次序：树那条路（`operator`）。与 `probe-watch-after` **并发**（两条边不互相等）。
pub static PROBE_WATCH_GONE: UnitFile = UnitFile {
    name: "probe-watch-gone",
    loader: crate::unit::LoaderConfig::user("probe-watch-gone"),
    operator: crate::unit::OperatorConfig { namespace: &[] },
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[],
    },
    control: crate::unit::ControlConfig {
        after: Some(&[crate::unit::Dependency::Bootstrap(
            crate::unit::Bootstrap::Operator,
        )]),
        ready: crate::unit::Ready::OnSpawn,
        ending: Some(Ending::Transient),

        construction: &[],
        retirement: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};
