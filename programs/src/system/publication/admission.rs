use alloc::{string::String, vec::Vec};
use system_api::control::publication::{Scope, Target};
use system_api::identity::Selector;
use system_api::operator::{PathBuf, Permit, name};
#[derive(Clone, Copy)]
pub(crate) enum Permission {
    Any,
    Public,
    Member,
}
pub(crate) struct Namespace {
    pub owner: &'static str,
    pub scope: Scope,
    pub group: String,
    pub road: PathBuf,
    pub entries: Option<Vec<String>>,
    pub permission: Permission,
    pub alias: bool,
}
pub(crate) struct Namespaces(pub Vec<Namespace>);
impl Namespaces {
    pub(crate) fn service(
        &self,
        owner: &str,
        (target, permit): (&Target, Permit),
    ) -> Option<(PathBuf, (bool, bool))> {
        let Target::Service {
            scope,
            group,
            name: leaf,
        } = target
        else {
            return None;
        };
        if !name::valid(leaf) {
            return None;
        }
        self.0.iter().find_map(|rule| {
            let member = matches!(rule.permission, Permission::Member);
            (rule.owner == owner
                && rule.scope == *scope
                && rule.group == *group
                && rule
                    .entries
                    .as_ref()
                    .is_none_or(|entries| entries.iter().any(|entry| entry == leaf))
                && match rule.permission {
                    Permission::Any => true,
                    Permission::Public => permit == Permit::Public,
                    Permission::Member => matches!(permit, Permit::Identity(Selector::MemberOf(_))),
                })
            .then(|| {
                rule.road
                    .try_join(leaf)
                    .map(|road| (road, (member, rule.alias)))
            })
            .flatten()
        })
    }
}
