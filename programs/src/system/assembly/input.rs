//! Construction inputs keep native image provenance separate from trusted Unit references.
use alloc::vec::Vec;
use env::TaskId;
use system_api::control::{
    Fail,
    construction::{Image, Input, UnitRef},
};

pub(crate) enum Source {
    Image { image: Image, from: TaskId },
    Unit(UnitRef),
}
impl Source {
    pub(crate) fn new(input: Input, from: TaskId) -> Self {
        match input {
            Input::Image(image) => Self::Image { image, from },
            Input::Unit(unit) => Self::Unit(unit),
        }
    }
    pub(crate) fn release(&self) {
        if let Self::Image { image, from } = self {
            crate::system::loader::release_image(image, *from);
        }
    }
    pub(crate) fn program(&self) -> Option<&'static crate::unit::UnitFile> {
        match self {
            Self::Image { .. } => None,
            Self::Unit(unit) => crate::unit::PROGRAMS
                .iter()
                .copied()
                .find(|program| program.name() == unit.name),
        }
    }
    pub(crate) fn parameters(&self) -> (u8, &[u64; system_api::loader::MAX_ARGS], u64) {
        match self {
            Self::Image { image, .. } => (image.count, &image.args, image.stack),
            Self::Unit(unit) => (unit.count, &unit.args, unit.stack),
        }
    }
    pub(crate) fn snapshot(
        &self,
        catalog: Option<crate::boot::Catalog<'static>>,
    ) -> Result<Vec<u8>, Fail> {
        match self {
            Self::Image { image, from } => {
                crate::system::loader::snapshot(crate::system::loader::Source {
                    from: *from,
                    ask: image,
                })
                .map_err(Into::into)
            }
            Self::Unit(_) => {
                let program = self.program().ok_or(Fail::Unknown)?;
                if !program.valid() {
                    return Err(Fail::Bad);
                }
                let source = catalog
                    .ok_or(Fail::NotReady)?
                    .find(program.loader.image)
                    .ok_or(Fail::BadImage)?
                    .elf;
                if source.is_empty() || source.len() > system_api::loader::MAX_IMAGE {
                    return Err(Fail::BadImage);
                }
                let mut bytes = Vec::new();
                bytes
                    .try_reserve_exact(source.len())
                    .map_err(|_| Fail::Full)?;
                bytes.extend_from_slice(source);
                Ok(bytes)
            }
        }
    }
}
