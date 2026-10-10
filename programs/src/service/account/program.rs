use crate::unit::{Demand, Ending, Identity, Relation, Setup, UnitFile};
pub static PROGRAM: UnitFile = UnitFile {
    publication: &[crate::unit::Publish::Entries {
        scope: crate::unit::PublishScope(4),
        group: "",
        road: "svc/account",
        entries: &[crate::unit::PublishEntry { name: "create" }],
        public: false,
    }],
    identity: Identity {
        name: "account",
        aliases: &["anran"],
        wanted_by: &["accept", "product", "system-fault"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "identity"]),
        restart: Some(Ending::Resident),
        ..Relation::DEFAULT
    },
    demand: Demand {
        construction: true,
        identity: &[system_api::identity::Grant::Derive],
        supply: &[
            Setup::Image {
                name: "shell",
                load: "account-image",
            },
            Setup::Image {
                name: "cat",
                load: "account-cat",
            },
            Setup::Image {
                name: "emit",
                load: "account-emit",
            },
            Setup::Image {
                name: "upper",
                load: "account-upper",
            },
            Setup::Image {
                name: "fail",
                load: "account-fail",
            },
            Setup::Image {
                name: "spin",
                load: "account-spin",
            },
            Setup::Image {
                name: "workers",
                load: "account-workers",
            },
            Setup::Ready,
        ],
    },
};
