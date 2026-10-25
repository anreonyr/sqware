use system_api::operator::{NamespaceKey, Ops, Permit};

use super::{Binding, Placement, Scope};

#[derive(Clone, Copy)]
pub struct Namespace {
    /// Logical namespace selected by product composition.
    pub key: NamespaceKey,
    /// Whether this declaration lives at the shared root or an instance prefix.
    pub placement: Placement,
    /// Namespace path relative to the selected entry and placement.
    pub path: &'static str,
    pub scope: Scope,
    pub ops: Ops,
    pub permit: Option<Permit>,
    pub source_mark: Option<u64>,
    pub bindings: &'static [Binding],
}

impl Namespace {
    pub fn valid(self) -> bool {
        if !system_api::operator::name::valid(self.key.name())
            || self.key.mark() != env::Mark::of(self.key.name())
            || (!self.path.is_empty() && !valid_path(self.path))
            || !self.ops.is_valid()
        {
            return false;
        }
        let scope_valid = match self.scope {
            Scope::Subtree => true,
            Scope::Names(names) => !names.is_empty() && names.iter().all(|name| valid_path(name)),
        };
        scope_valid
            && self.bindings.iter().enumerate().all(|(index, binding)| {
                system_api::operator::name::valid(binding.name)
                    && binding.mark != env::Mark::NONE
                    && self.bindings[..index]
                        .iter()
                        .all(|previous| previous.name != binding.name)
                    && binding.view != env::Access::NONE
            })
    }
}

fn valid_path(path: &str) -> bool {
    let Some(normalized) = system_api::operator::PathBuf::try_new(path) else {
        return false;
    };
    !normalized.is_empty()
        && normalized.as_str() == path
        && path.split('/').all(system_api::operator::name::valid)
}
