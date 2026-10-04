use crate::lock::OnceLock;
use crate::resource::Registry;
use crate::work::mail::nole::NoleMeta;
use crate::work::unit::gate::{self, AnyPie};
use alloc::sync::Arc;
use env::{Call, Mark, Name, Permission, PieFail, TaskId};

pub(crate) struct Resources {
    pub build: Arc<NoleMeta>,
}
static RESOURCES: OnceLock<Resources> = OnceLock::new();

pub(crate) fn register(registry: &mut Registry) -> Result<(), PieFail> {
    let build = &get().build;
    let root = gate::try_new_pie(
        build.clone(),
        Mark::NONE,
        Permission::FETCH | Permission::VEST,
        None,
    )?;
    registry
        .register(Name::Call(Call::Build), AnyPie::Nole(root))
        .map_err(|e| e.into_parts().0)?;
    Ok(())
}
pub(crate) fn init() {
    RESOURCES.get_or_init(|| Resources {
        build: NoleMeta::new(TaskId::new(0)),
    });
}
pub(crate) fn get() -> &'static Resources {
    RESOURCES.get().expect("call resources not initialized")
}
