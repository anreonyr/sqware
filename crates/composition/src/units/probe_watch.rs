//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! `units/mod.rs` 的 `PROGRAMS`）。

use crate::unit::{Ending, UnitFile};

/// **事件那条路的正证客人**：订一条子树，再自己往树上落两格（一棵在订的范围里、一棵不在），
/// 判据是"**订的那一棵到了、另一棵没到**"，且事件里那条路与那一号与树上对得上。
///
/// 自持两位（`watch` ＋ `land`）＝**两条会话**：一位客人一条会话是本族既有的形状（认领键是
/// "谁开的 ＋ 树路记号"）。故这一台不借别人铺试验场——它落的每一步都由它自己报得出坐标。
///
/// 次序：树那条路（`operator`）；它只跟树说话。
pub static PROBE_WATCH: UnitFile = UnitFile {
    name: "probe-watch",
    loader: crate::unit::LoaderConfig::user("probe-watch"),
    operator: crate::unit::OperatorConfig {
        namespace: &[
            crate::unit::operator::Namespace {
                key: system_api::namespace::SERVICE,
                placement: crate::unit::operator::Placement::Shared,
                path: "probe-watch",
                scope: crate::unit::operator::Scope::Names(&["in", "out"]),
                ops: system_api::operator::Ops::BIND
                    .union(system_api::operator::Ops::UNBIND)
                    .union(system_api::operator::Ops::FIND),
                permit: None,
                source_mark: None,
                bindings: &[],
            },
            crate::unit::operator::Namespace {
                key: system_api::namespace::SERVICE,
                placement: crate::unit::operator::Placement::Shared,
                path: "probe-watch-dynamic",
                scope: crate::unit::operator::Scope::Subtree,
                ops: system_api::operator::Ops::BIND
                    .union(system_api::operator::Ops::UNBIND)
                    .union(system_api::operator::Ops::FIND),
                permit: None,
                source_mark: Some(env::Mark::of("probe-watch-resource").get()),
                bindings: &[],
            },
            crate::unit::operator::Namespace {
                key: system_api::namespace::SERVICE,
                placement: crate::unit::operator::Placement::Shared,
                path: "probe-watch-q",
                scope: crate::unit::operator::Scope::Names(&["c0", "c1", "c2", "c3", "c4", "c5"]),
                ops: system_api::operator::Ops::BIND
                    .union(system_api::operator::Ops::UNBIND)
                    .union(system_api::operator::Ops::FIND),
                permit: None,
                source_mark: None,
                bindings: &[],
            },
        ],
    },
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[],
    },
    control: crate::unit::ControlConfig {
        supervision: None,
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
