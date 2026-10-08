use crate::unit::{Publish, PublishScope};
use system_api::control::publication::{Scope, Target};
use system_api::identity::Selector;
use system_api::operator::{PathBuf, Permit, name};

pub(super) fn service(
    rules: &[Publish],
    target: &Target,
    permit: Permit,
) -> Option<PathBuf> {
    let Target::Service { scope, group, name: leaf } = target else {
        return None;
    };
    if !name::valid(leaf) {
        return None;
    }
    rules.iter().find_map(|rule| {
        let (allowed, expected, road, public, listed) = match rule {
            Publish::Entries {
                scope,
                group,
                road,
                entries,
                public,
            } => {
                let listed = entries.iter().any(|entry| entry.name == leaf);
                (*scope, *group, *road, *public, listed)
            }
            Publish::Namespace {
                scope,
                group,
                road,
                public,
            } => (*scope, *group, *road, *public, true),
            Publish::Devices => return None,
        };
        (to_scope(allowed) == *scope
            && expected == group.as_str()
            && listed
            && (!public || permit == Permit::Public))
            .then(|| system_api::operator::Path::new(road).try_join(leaf))
            .flatten()
    })
}

pub(super) fn devices(rules: &[Publish], trusted_hub: bool, (scope, permit): (Scope, Permit)) -> bool {
    trusted_hub
        && scope == Scope::Device
        && matches!(permit, Permit::Identity(Selector::MemberOf(_)))
        && rules.iter().any(|rule| matches!(rule, Publish::Devices))
}

const fn to_scope(scope: PublishScope) -> Scope {
    match scope {
        PublishScope::Driver => Scope::Driver,
        PublishScope::Hub => Scope::Hub,
        PublishScope::Fixture => Scope::Fixture,
        PublishScope::Terminal => Scope::Terminal,
    }
}
