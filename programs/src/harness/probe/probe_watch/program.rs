//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! `../unit/catalog.rs` 的 `PROGRAMS`）。

use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

/// **事件那条路的正证客人**：订一条子树，再自己往树上落两格（一棵在订的范围里、一棵不在），
/// 判据是"**订的那一棵到了、另一棵没到**"，且事件里那条路与那一号与树上对得上。
///
/// 自持两位（`watch` ＋ `land`）＝**两条会话**：一位客人一条会话是本族既有的形状（认领键是
/// "谁开的 ＋ 树路记号"）。故这一台不借别人铺试验场——它落的每一步都由它自己报得出坐标。
///
/// 次序：树那条路（`operator`）；它只跟树说话。
pub static PROBE_WATCH: UnitFile = UnitFile {
    publication: &[
        crate::unit::Publish::Entries {
            scope: crate::unit::PublishScope::Fixture,
            group: "probe-watch",
            road: "svc/probe-watch",
            entries: &[
                crate::unit::PublishEntry { name: "in" },
                crate::unit::PublishEntry { name: "out" },
            ],
            public: false,
        },
        crate::unit::Publish::Namespace {
            scope: crate::unit::PublishScope::Fixture,
            group: "probe-watch-dynamic",
            road: "svc/probe-watch-dynamic",
            public: false,
        },
        crate::unit::Publish::Entries {
            scope: crate::unit::PublishScope::Fixture,
            group: "probe-watch-q",
            road: "svc/probe-watch-q",
            entries: &[
                crate::unit::PublishEntry { name: "c0" },
                crate::unit::PublishEntry { name: "c1" },
                crate::unit::PublishEntry { name: "c2" },
                crate::unit::PublishEntry { name: "c3" },
                crate::unit::PublishEntry { name: "c4" },
                crate::unit::PublishEntry { name: "c5" },
            ],
            public: false,
        },
    ],
    identity: Identity {
        name: "probe-watch",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
