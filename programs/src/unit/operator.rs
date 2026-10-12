//! Static Operator declarations consumed by System composition.

use env::Access;
use system_api::operator::{Ops, Permit};

#[derive(Clone, Copy)]
pub struct Namespace {
    /// Namespace path relative to the System-selected shared directory.
    pub path: &'static str,
    pub scope: Scope,
    pub ops: Ops,
    pub permit: Option<Permit>,
    pub source_mark: Option<u64>,
    pub bindings: &'static [Binding],
}

#[derive(Clone, Copy)]
pub enum Scope {
    Subtree,
    Names(&'static [&'static str]),
}

#[derive(Clone, Copy)]
pub struct Binding {
    pub name: &'static str,
    /// Locator only; composition must validate the resolved live resource before binding it.
    pub mark: env::Mark,
    pub view: Access,
    pub permit: Permit,
}

impl Namespace {
    pub fn valid(self) -> bool {
        if (!self.path.is_empty() && !valid_path(self.path)) || !self.ops.is_valid() {
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
                    && binding.view != Access::NONE
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
