//! Pure executable-image configuration owned by each UnitFile.

use env::ProgramKind;

#[derive(Clone, Copy)]
pub struct LoaderConfig {
    pub image: &'static str,
    pub space: ProgramKind,
    pub args: &'static [usize],
    pub stack: usize,
}

impl LoaderConfig {
    pub const fn user(image: &'static str) -> Self {
        Self {
            image,
            space: ProgramKind::User,
            args: &[],
            stack: 0,
        }
    }

    pub const fn supervisor(image: &'static str) -> Self {
        Self {
            image,
            space: ProgramKind::Supervisor,
            args: &[],
            stack: 0,
        }
    }
}
