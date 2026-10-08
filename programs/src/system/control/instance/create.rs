//! Instance creation and image-cache ownership.

use crate::system::control::unit::Control;
use crate::system::loader::{Image, serve::build::Spawn};
use env::TaskId;
use system_api::{control::Fail, loader::Built};

pub(crate) struct Creation<'a> {
    pub image: Image<'a>,
    pub spawn: Spawn<'a>,
    pub owner: TaskId,
}

impl Control {
    pub(crate) fn create_instance(&mut self, creation: Creation<'_>) -> Result<Built, Fail> {
        self.reserve_instance()?;
        let built = crate::system::loader::serve::build::construct(
            &mut self.loader,
            creation.image,
            creation.spawn,
        )?;
        self.register_instance(built, creation.owner);
        Ok(built)
    }

    pub(crate) fn clear_images(&mut self) {
        self.loader.clear();
    }
}
