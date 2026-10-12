//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! `../unit/catalog.rs` 的 `PROGRAMS`）。

use crate::unit::{Ending, UnitFile};

/// **"退场即撤订"那一对的后半**：在同一条路上连改几趟（见 `main.rs` 头注）。
///
/// 次序：树那条路（`operator`）。**不声明等 `probe-watch-gone`**：`after` 只等到"答得动"，
/// 等不到"退场"——两台的判据靠"连改跨过那一刻"成立。
pub static PROBE_WATCH_AFTER: UnitFile = UnitFile {
    name: "probe-watch-after",
    loader: crate::unit::LoaderConfig::user("probe-watch-after"),
    operator: crate::unit::OperatorConfig {
        namespace: &[crate::unit::operator::Namespace {
            key: system_api::namespace::SERVICE,
            placement: crate::unit::operator::Placement::Shared,
            path: "probe-swatch",
            scope: crate::unit::operator::Scope::Names(&["in"]),
            ops: system_api::operator::Ops::BIND
                .union(system_api::operator::Ops::UNBIND)
                .union(system_api::operator::Ops::FIND),
            permit: None,
            source_mark: None,
            bindings: &[],
        }],
    },
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
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};
