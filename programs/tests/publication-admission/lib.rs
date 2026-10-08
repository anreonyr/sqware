#![allow(dead_code)]

#[path = "../../src/unit/publication.rs"]
mod publication_metadata;

mod unit {
    pub use crate::publication_metadata::{Publish, PublishScope};
}

#[path = "../../src/system/publication/admission.rs"]
mod admission;

#[cfg(test)]
mod tests {
    use super::admission;
    use super::publication_metadata::{Publish, PublishEntry, PublishScope};
    use env::TaskId;
    use system_api::control::publication::Scope;
    use system_api::control::publication::Target;
    use system_api::identity::{CoalitionId, Selector};
    use system_api::operator::{Permit, path::PathBuf};

    fn target(scope: Scope, group: &str, name: &str) -> Target {
        Target::Service {
            scope,
            group: group.into(),
            name: name.into(),
        }
    }

    #[test]
    fn namespace_admits_distinct_markless_names_only_inside_its_declared_path() {
        let rules = [Publish::Namespace {
            scope: PublishScope::Fixture,
            group: "dynamic-fixture",
            road: "svc/dynamic-fixture",
            public: false,
        }];
        for (name, expected) in [
            ("first", "svc/dynamic-fixture/first"),
            ("second", "svc/dynamic-fixture/second"),
        ] {
            assert_eq!(
                admission::service(
                    &rules,
                    &target(Scope::Fixture, "dynamic-fixture", name),
                    Permit::Public,
                ),
                PathBuf::try_new(expected),
            );
        }
        assert!(
            admission::service(
                &rules,
                &target(Scope::Driver, "dynamic-fixture", "first"),
                Permit::Public,
            )
            .is_none()
        );
        assert!(
            admission::service(
                &rules,
                &target(Scope::Fixture, "other-group", "first"),
                Permit::Public,
            )
            .is_none()
        );
        assert!(
            admission::service(
                &rules,
                &target(Scope::Fixture, "dynamic-fixture", "../escape"),
                Permit::Public,
            )
            .is_none()
        );
    }

    #[test]
    fn fixed_names_and_public_permit_policy_remain_bounded() {
        static ENTRIES: [PublishEntry; 1] = [PublishEntry { name: "fixed" }];
        let rules = [Publish::Entries {
            scope: PublishScope::Fixture,
            group: "fixed-fixture",
            road: "svc/fixed-fixture",
            entries: &ENTRIES,
            public: true,
        }];
        assert!(
            admission::service(
                &rules,
                &target(Scope::Fixture, "fixed-fixture", "fixed"),
                Permit::Public,
            )
            .is_some()
        );
        assert!(
            admission::service(
                &rules,
                &target(Scope::Fixture, "fixed-fixture", "another"),
                Permit::Public,
            )
            .is_none()
        );
        assert!(
            admission::service(
                &rules,
                &target(Scope::Fixture, "fixed-fixture", "fixed"),
                Permit::Bound,
            )
            .is_none()
        );
    }

    #[test]
    fn device_namespace_requires_trusted_hub_and_member_of_permit() {
        let rules = [Publish::Devices];
        let member = Permit::Identity(Selector::MemberOf(CoalitionId::new(TaskId::new(1), 2)));
        assert!(admission::devices(&rules, true, (Scope::Device, member)));
        assert!(!admission::devices(
            &rules,
            false,
            (Scope::Device, member)
        ));
        assert!(!admission::devices(
            &rules,
            true,
            (Scope::Device, Permit::Public)
        ));
        assert!(!admission::devices(&rules, true, (Scope::Hub, member)));
    }
}
