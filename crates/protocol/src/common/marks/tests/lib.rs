#![allow(dead_code)]
#[path = "../../marks.rs"]
pub mod definitions;
mod common { pub use crate::definitions as marks; }
#[path = "../../../system/control/marks.rs"]
mod control;
#[path = "../../../system/identity/marks.rs"]
mod identity;
#[path = "../../../system/operator/marks.rs"]
mod operator;
#[path = "../../../system/loader/marks.rs"]
pub mod loader;
#[path = "../../../driver/marks.rs"]
mod driver;
#[path = "../../../service/hub/marks.rs"]
mod hub;
#[path = "../../../service/terminal/marks.rs"]
mod terminal;

#[cfg(test)]
mod tests {
    use super::*;
    use definitions::{Definition, conflict};
    #[test]
    fn declared_domains_are_unique_and_existing_values_are_preserved() {
        let groups = [control::DECLARATIONS, &identity::DECLARATIONS, &operator::DECLARATIONS,
            loader::DECLARATIONS, driver::DECLARATIONS, hub::DECLARATIONS, terminal::DECLARATIONS];
        assert_eq!(groups.iter().map(|group| group.len()).sum::<usize>(), 29);
        assert_eq!(conflict(&groups), None);
        for definition in groups.iter().flat_map(|group| group.iter()) {
            assert_eq!(definition.mark, env::Mark::of(definition.name));
            assert_ne!(definition.mark, env::Mark::NONE);
        }
        assert_eq!(control::PUBLICATION_ENTRY, env::Mark::of("control-publication"));
        assert_eq!(control::PUBLICATION_BACK, env::Mark::of("control-publication-back"));
        assert_eq!(control::IDENTITY_REF, env::Mark::of("control-identity-ref"));
        assert_eq!(driver::LINE_BACK, env::Mark::of("line-back"));
        assert_eq!(operator::TIP_MARK, env::Mark::of("tip"));
    }
    #[test]
    fn collision_reports_both_names_across_or_within_groups() {
        let a = [Definition { name: "a", mark: env::Mark::of("same") }];
        let b = [Definition { name: "b", mark: a[0].mark }];
        assert_eq!(conflict(&[&a, &[], &b]), Some(("a", "b")));
        assert_eq!(conflict(&[&[a[0], b[0]]]), Some(("a", "b")));
        assert_eq!(conflict(&[]), None);
    }
    #[test]
    fn publication_marks_are_checked_against_other_domains() {
        let collision = [Definition { name: "foreign", mark: control::PUBLICATION_BACK }];
        assert_eq!(conflict(&[control::DECLARATIONS, &collision]),
            Some(("control-publication-back", "foreign")));
    }
}
