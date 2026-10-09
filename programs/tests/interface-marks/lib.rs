#![allow(dead_code)]

#[path = "../../src/unit/interfaces.rs"]
mod interfaces;

mod baseline;

#[cfg(test)]
mod tests {
    use env::marks::Definition;

    #[test]
    fn every_provider_mark_is_unique_and_keeps_its_value() {
        use system_api::{control, identity, operator};
        let groups: [&[Definition]; 7] = [
            &control::marks::DECLARATIONS,
            &identity::marks::DECLARATIONS,
            &operator::marks::DECLARATIONS,
            system_api::loader::CHANNELS,
            router_api::marks::DECLARATIONS,
            hub_api::marks::DECLARATIONS,
            terminal_api::marks::DECLARATIONS,
        ];
        assert_eq!(groups.iter().map(|group| group.len()).sum::<usize>(), 26);
        assert_eq!(env::marks::conflict(&groups), None);
        for definition in groups.iter().flat_map(|group| group.iter()) {
            assert_eq!(definition.mark, env::Mark::of(definition.name));
            assert_ne!(definition.mark, env::Mark::NONE);
        }
        assert_eq!(control::marks::PUBLICATION_ENTRY, env::Mark::of("control-publication"));
        assert_eq!(control::marks::PUBLICATION_BACK, env::Mark::of("control-publication-back"));
        assert_eq!(control::marks::IDENTITY_REF, env::Mark::of("control-identity-ref"));
        assert_eq!(router_api::marks::LINE_BACK, env::Mark::of("line-back"));
        assert_eq!(operator::marks::TIP_MARK, env::Mark::of("tip"));
    }

    #[test]
    fn all_registered_roles_keep_the_fixed_legacy_values() {
        let actual: Vec<_> = crate::interfaces::APIS.iter()
            .flat_map(|registry| registry.iter())
            .flat_map(|group| group.iter())
            .collect();
        assert_eq!(actual.len(), 55);
        assert_eq!(actual.len() - 3 + crate::baseline::RETIRED.len(), crate::baseline::VALUES.len());
        for &(name, value) in crate::baseline::VALUES {
            let matches: Vec<_> = actual.iter().filter(|definition| definition.name == name).collect();
            if crate::baseline::RETIRED.contains(&name) {
                assert!(matches.is_empty(), "retired role reappeared: {name}");
                assert!(!actual.iter().any(|definition| definition.mark.get() == value),
                    "retired role value reused: {name}");
                continue;
            }
            assert_eq!(matches.len(), 1, "role missing or duplicated: {name}");
            assert_eq!(matches[0].mark.get(), value, "legacy value changed: {name}");
        }
    }

    #[test]
    fn assembled_provider_registries_have_no_collisions() {
        assert_eq!(env::marks::conflict_between(crate::interfaces::APIS), None);
    }

    #[test]
    fn collision_reports_pairs_within_and_across_registries() {
        let a = [Definition { name: "a", mark: env::Mark::of("same") }];
        let b = [Definition { name: "b", mark: a[0].mark }];
        let a_group: &[&[Definition]] = &[&a];
        let b_group: &[&[Definition]] = &[&b];
        assert_eq!(env::marks::conflict_between(&[a_group, b_group]), Some(("a", "b")));

        let together = [a[0], b[0]];
        let one_group: &[&[Definition]] = &[&together];
        assert_eq!(env::marks::conflict_between(&[one_group]), Some(("a", "b")));
        assert_eq!(env::marks::conflict_between(&[]), None);
    }

    #[test]
    fn publication_marks_are_checked_against_other_provider_registries() {
        let foreign = [Definition {
            name: "foreign",
            mark: system_api::control::marks::PUBLICATION_BACK,
        }];
        let foreign_registry: &[&[Definition]] = &[&foreign];
        assert_eq!(
            env::marks::conflict_between(&[system_api::control::REGISTRY, foreign_registry]),
            Some(("control-publication-back", "foreign")),
        );
    }
}
