use crate::{GuardSelectionError, UnitFile, guard_selection};

mod bootstrap;
mod dependency;
mod scene;
mod select;
mod target;

pub use bootstrap::Bootstrap;
pub use dependency::Dependency;
pub use scene::SCENE;
pub use select::select;
pub use target::Target;

#[derive(Clone, Copy)]
pub struct Deployment {
    pub name: &'static str,
    pub entry: &'static str,
    pub start: &'static [&'static str],
    pub guarded: &'static [&'static str],
    pub resources: &'static [&'static str],
    pub targets: &'static [Target],
    pub bootstrap: &'static [Bootstrap],
    pub sources: &'static [&'static str],
}

impl Deployment {
    pub fn find<'a>(deployments: &'a [Deployment], name: &str) -> Option<&'a Self> {
        deployments
            .iter()
            .find(|deployment| deployment.name == name)
    }

    pub fn entry<'a>(&self, units: &'a [&'static UnitFile]) -> Option<&'a UnitFile> {
        units
            .iter()
            .copied()
            .find(|unit| unit.name() == self.entry)
    }

    pub fn entry_image(&self, units: &[&'static UnitFile]) -> Option<&'static str> {
        self.entry(units).map(|unit| unit.loader.image)
    }

    pub fn guarded(
        &self,
        units: &[&'static UnitFile],
    ) -> Result<&'static [&'static str], GuardSelectionError> {
        guard_selection::select(self.guarded, self.start, |name| {
            units.iter().any(|unit| unit.name() == name)
        })
    }
}

/// A deployment's scene target is the shared scene name.
pub const fn scene_target(members: &'static [&'static str]) -> Target {
    Target {
        name: SCENE,
        members,
    }
}
