#![allow(dead_code)]
extern crate alloc;
extern crate self as system_client;

#[path = "system/operator/mod.rs"]
mod operator;

pub mod identity {
    pub struct TaskQuery;

    impl TaskQuery {
        pub fn available(&self) -> bool {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;
    use env::{Access, Mark, PieToken, TaskId};
    use crate::operator::tree::{BindEntry, Operator};
    use system_api::operator::{
        Entry, EntryId, EntryTarget, Fail, NamespaceKey, Path, PathBuf, Permit, Scope, Where,
    };

    const INTERFACE: NamespaceKey = NamespaceKey::of("identity.interface");

    fn new_operator(actor: usize) -> Operator {
        let system = TaskId::new(1);
        let mut operator = Operator::new(system, TaskId::new(actor));
        operator
            .bootstrap_root(system, PieToken::mint(actor + 100))
            .unwrap();
        operator
    }

    fn part(operator: &mut Operator, parent: Entry, name: &str) -> Entry {
        operator
            .part_entry(parent, name.to_string())
            .unwrap()
            .0
    }

    fn reference(operator: &Operator, entry: Entry) -> Entry {
        operator
            .seek_from(entry, Path::new("interface"))
            .unwrap()
    }

    #[test]
    fn stable_key_resolves_to_the_module_entry_across_distinct_layouts() {
        let mut first = new_operator(2);
        let root = first.entry(Where::Root).unwrap();
        let service = part(&mut first, root, "service");
        let first_identity = part(&mut first, service, "identity");
        let first_interface = part(&mut first, first_identity, "interface");
        let login = part(&mut first, first_interface, "login");
        let first_private = part(&mut first, first_identity, "private");
        first
            .associate_namespace(INTERFACE, first_identity)
            .unwrap();

        let mut second = new_operator(3);
        let root = second.entry(Where::Root).unwrap();
        let unit = part(&mut second, root, "unit");
        let contracts = part(&mut second, unit, "contracts");
        let second_identity = part(&mut second, contracts, "identity");
        let second_interface = part(&mut second, second_identity, "interface");
        let _ = part(&mut second, second_interface, "login");
        second
            .associate_namespace(INTERFACE, second_identity)
            .unwrap();

        let first_resolved = first.resolve_namespace(INTERFACE).unwrap();
        let second_resolved = second.resolve_namespace(INTERFACE).unwrap();
        assert_eq!(first_resolved, first_identity);
        assert_eq!(second_resolved, second_identity);
        assert_eq!(reference(&first, first_resolved), first_interface);
        assert_eq!(reference(&second, second_resolved), second_interface);
        assert_ne!(first_resolved, second_resolved);

        let interface_scope = Scope::Names(alloc::vec![PathBuf::try_new("interface").unwrap()]);
        let login_path = first.relative_to(first_resolved, login).unwrap();
        let private_path = first.relative_to(first_resolved, first_private).unwrap();
        assert_eq!(login_path.as_str(), "interface/login");
        assert!(interface_scope.allows(login_path.as_str()));
        assert!(!interface_scope.allows(private_path.as_str()));
    }

    #[test]
    fn same_key_registration_is_idempotent_and_conflicting_entry_is_rejected() {
        let mut operator = new_operator(4);
        let root = operator.entry(Where::Root).unwrap();
        let identity = part(&mut operator, root, "identity");
        let other = part(&mut operator, root, "other");

        assert_eq!(operator.associate_namespace(INTERFACE, identity), Ok(()));
        assert_eq!(operator.associate_namespace(INTERFACE, identity), Ok(()));
        assert_eq!(operator.resolve_namespace(INTERFACE), Ok(identity));
        assert_eq!(
            operator.associate_namespace(INTERFACE, other),
            Err(Fail::Conflict)
        );
        assert_eq!(operator.resolve_namespace(INTERFACE), Ok(identity));
    }

    #[test]
    fn association_requires_a_live_directory_from_this_operator() {
        let mut operator = new_operator(7);
        let root = operator.entry(Where::Root).unwrap();
        let directory = part(&mut operator, root, "identity");

        let foreign_root = new_operator(8).entry(Where::Root).unwrap();
        assert_eq!(
            operator.associate_namespace(NamespaceKey::of("foreign"), foreign_root),
            Err(Fail::Denied)
        );

        let entrance = PieToken::mint(107);
        let authority = operator.authority_id(entrance).unwrap();
        let tile = operator
            .bind_at(BindEntry {
                anchor: root,
                name: "resource".into(),
                pie: PieToken::mint(700),
                resource_id: 1,
                permit: Permit::Public,
                owner: TaskId::new(1),
                authority,
                view: Access::FETCH,
            })
            .unwrap()
            .0
            .entry;
        assert_ne!(tile, directory);
        assert_eq!(
            operator.associate_namespace(NamespaceKey::of("tile.namespace"), tile),
            Err(Fail::NotAPane)
        );
        assert_eq!(
            operator.seek_from(tile, &Path::new("")),
            Err(Fail::NotAPane)
        );
        assert_eq!(
            operator.resolve_namespace(NamespaceKey::of("tile.namespace")),
            Err(Fail::Unknown)
        );
    }

    #[test]
    fn empty_relative_path_preserves_root_and_node_zero_as_distinct_entries() {
        let mut operator = new_operator(9);
        let root = operator.entry(Where::Root).unwrap();
        let node_zero = part(&mut operator, root, "first");
        assert_eq!(node_zero.target, EntryTarget::Node(EntryId::new(0)));
        let empty = Path::new("");

        assert_eq!(operator.seek_from(root, &empty), Ok(root));
        assert_eq!(operator.seek_from(node_zero, &empty), Ok(node_zero));
        assert_ne!(
            operator.seek_from(root, &empty),
            operator.seek_from(node_zero, &empty)
        );
    }

    #[test]
    fn namespace_registry_is_bounded_and_full_replay_stays_idempotent() {
        let mut operator = new_operator(10);
        let root = operator.entry(Where::Root).unwrap();
        let anchor = part(&mut operator, root, "layout");
        let conflict = part(&mut operator, root, "other");
        let mut cursor = anchor;
        let mut first_key = None;

        for index in 0..256 {
            let name = alloc::format!("layout.key.{index}");
            let mark = Mark::of(&name);
            crate::operator::associate_parts(&mut operator, mark, &name, anchor).unwrap();
            if index == 0 {
                first_key = Some(name.clone());
            }
            cursor = part(&mut operator, cursor, "next");
        }

        let first_key = first_key.unwrap();
        let first_mark = Mark::of(&first_key);
        assert_eq!(
            crate::operator::associate_parts(&mut operator, first_mark, &first_key, anchor),
            Ok(())
        );
        assert_eq!(
            crate::operator::associate_parts(&mut operator, first_mark, &first_key, conflict),
            Err(Fail::Conflict)
        );
        assert_eq!(
            operator.resolve_namespace_parts(first_mark, &first_key),
            Ok(anchor)
        );

        let overflow = "layout.key.overflow";
        assert_eq!(
            crate::operator::associate_parts(&mut operator, Mark::of(overflow), overflow, anchor),
            Err(Fail::Full)
        );
        assert_eq!(
            operator.resolve_namespace_parts(Mark::of(overflow), overflow),
            Err(Fail::Unknown)
        );
        assert_ne!(cursor, anchor);
    }

    #[test]
    fn unknown_key_does_not_fall_back_to_root_or_another_registered_entry() {
        let mut operator = new_operator(5);
        let root = operator.entry(Where::Root).unwrap();
        let fallback = part(&mut operator, root, "identity");
        operator.associate_namespace(INTERFACE, fallback).unwrap();

        assert_eq!(
            operator.resolve_namespace(NamespaceKey::of("identity.unknown")),
            Err(Fail::Unknown)
        );
        assert_ne!(fallback.target, EntryTarget::Root);
    }

    #[test]
    fn stale_association_stays_unknown_after_same_path_is_recreated() {
        let mut operator = new_operator(6);
        let root = operator.entry(Where::Root).unwrap();
        let stale = part(&mut operator, root, "identity");
        operator.associate_namespace(INTERFACE, stale).unwrap();

        operator.unpart(stale).unwrap();
        assert_eq!(operator.resolve_namespace(INTERFACE), Err(Fail::Unknown));
        assert_eq!(operator.seek_from(stale, &Path::new("")), Err(Fail::Unknown));
        assert_eq!(
            operator.associate_namespace(INTERFACE, stale),
            Err(Fail::Unknown)
        );

        let replacement = part(&mut operator, root, "identity");
        assert_ne!(replacement, stale);
        assert_eq!(
            operator.associate_namespace(INTERFACE, replacement),
            Err(Fail::Conflict)
        );
        assert_eq!(operator.resolve_namespace(INTERFACE), Err(Fail::Unknown));
    }

    #[test]
    fn removing_one_instance_leaf_preserves_the_shared_team_and_sibling() {
        let mut operator = new_operator(12);
        let root = operator.entry(Where::Root).unwrap();
        let unit = part(&mut operator, root, "unit");
        let team = part(&mut operator, unit, "7");
        let first_task = part(&mut operator, team, "11");
        let second_task = part(&mut operator, team, "12");

        assert!(operator.unpart(first_task).unwrap().is_some());
        assert_eq!(operator.seek_from(unit, &Path::new("7")), Ok(team));
        assert_eq!(operator.seek_from(team, &Path::new("12")), Ok(second_task));
        assert_eq!(operator.seek_from(team, &Path::new("11")), Err(Fail::Unknown));
    }
}
