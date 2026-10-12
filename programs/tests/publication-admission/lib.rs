extern crate alloc;
#[path = "../../src/unit/operator.rs"]
mod declaration;
#[cfg(test)]
mod tests {
    use super::declaration::{Namespace, Scope};
    use system_api::{operator::{Ops, Permit, Scope as GrantedScope, Path}, identity::{CoalitionId, Selector}};
    fn rules(scope: Scope, permit: Option<Permit>) -> Namespace {
        Namespace { key: system_api::namespace::SERVICE, placement: super::declaration::Placement::Shared, path: "arbitrary/layout", scope, ops: Ops::BIND.union(Ops::FIND), permit, source_mark: None, bindings: &[] }
    }
    #[test]
    fn composition_controls_layout_and_bounded_names_without_service_classes() {
        let policy = rules(Scope::Names(&["first", "second"]), None);
        assert!(policy.valid());
        assert_eq!(policy.path, "arbitrary/layout");
        let scope = GrantedScope::Names(alloc::vec![Path::new("first").to_path_buf(), Path::new("second").to_path_buf()]);
        assert!(scope.allows("first"));
        assert!(!scope.allows("unlisted"));
        assert!(!scope.allows("../escape"));
        let mut invalid = policy;
        invalid.path = "../escape";
        assert!(!invalid.valid());
    }
    #[test]
    fn finite_grants_can_narrow_but_cannot_escape_the_installed_scope() {
        let parent = GrantedScope::Names(alloc::vec![Path::new("first").to_path_buf(), Path::new("second").to_path_buf()]);
        assert!(parent.narrows(&GrantedScope::Names(alloc::vec![Path::new("first").to_path_buf()])));
        assert!(!parent.narrows(&GrantedScope::Names(alloc::vec![Path::new("third").to_path_buf()])));
        assert!(!parent.narrows(&GrantedScope::Subtree));
    }
    #[test]
    fn permission_choice_is_explicit_and_shared_base_requires_no_extra_directory() {
        let member = Permit::Identity(Selector::MemberOf(CoalitionId::new(env::TaskId::new(1), 0)));
        for permit in [None, Some(Permit::Public), Some(Permit::Bound), Some(member)] {
            let mut policy = rules(Scope::Subtree, permit);
            assert!(policy.valid());
            assert_eq!(policy.permit, permit);
            policy.path = "";
            assert!(policy.valid(), "explicit shared base is valid");
        }
        assert!(!rules(Scope::Names(&[]), None).valid());
    }
}
