extern crate alloc;
#[path = "../../src/system/publication/admission.rs"]
mod admission;
#[cfg(test)]
mod tests {
    use super::admission::*;
    use system_api::{control::publication::{Scope, Target}, operator::{Path, Permit}, identity::{CoalitionId, Selector}};
    fn target(name: &str) -> Target { Target::Service { scope: Scope(6), group: "custom".into(), name: name.into() } }
    fn rules(permission: Permission) -> Namespaces { Namespaces(vec![Namespace { owner: "provider", scope: Scope(6), group: "custom".into(), road: Path::new("arbitrary/layout").to_path_buf(), entries: Some(vec!["first".into(), "second".into()]), permission, alias: false }]) }
    #[test]
    fn deployment_controls_namespace_and_layout_without_builtin_service_classes() {
        let rules = rules(Permission::Any);
        assert_eq!(rules.service("provider", (&target("first"), Permit::Public)).unwrap().0.as_str(), "arbitrary/layout/first");
        assert!(rules.service("other", (&target("first"), Permit::Public)).is_none());
        assert!(rules.service("provider", (&target("unlisted"), Permit::Public)).is_none());
        assert!(rules.service("provider", (&target("../escape"), Permit::Public)).is_none());
        let mut wrong = target("first"); if let Target::Service { scope, .. } = &mut wrong { *scope = Scope(7); }
        assert!(rules.service("provider", (&wrong, Permit::Public)).is_none());
    }
    #[test]
    fn member_namespaces_reject_public_permissions_and_keep_whitelists() {
        let rules = rules(Permission::Member);
        let permit = Permit::Identity(Selector::MemberOf(CoalitionId::new(env::TaskId::new(1), 0)));
        assert!(rules.service("provider", (&target("first"), permit)).unwrap().1.0);
        assert!(rules.service("provider", (&target("third"), permit)).is_none());
        assert!(rules.service("provider", (&target("first"), Permit::Public)).is_none());
    }
    #[test]
    fn public_and_open_namespace_policies_remain_explicit() {
        let mut rules = rules(Permission::Public);
        assert!(rules.service("provider", (&target("first"), Permit::Bound)).is_none());
        rules.0[0].entries = None;
        assert!(rules.service("provider", (&target("third"), Permit::Public)).is_some());
    }
}
