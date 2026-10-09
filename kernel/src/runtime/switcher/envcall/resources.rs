use crate::lock::OnceLock;
use crate::resource::Registry;
use crate::work::mail::nole::NoleMeta;
use crate::work::unit::gate;
use alloc::sync::Arc;
use env::{Call, Mark, Name, Permission, PieFail, TaskId};

pub(crate) struct Resources {
    pub build: Arc<NoleMeta>,
    pub doom: Arc<NoleMeta>,
}
static RESOURCES: OnceLock<Resources> = OnceLock::new();

pub(crate) fn register(registry: &mut Registry) -> Result<(), PieFail> {
    for (call, meta) in [(Call::Build, &get().build), (Call::Doom, &get().doom)] {
        let root = gate::try_new_pie::<gate::Nole>(
            meta.clone(), Mark::NONE, Permission::FETCH | Permission::VEST, None,
        )?;
        registry.register(Name::Call(call), gate::boxed(root)?)
            .map_err(|e| e.into_parts().0)?;
    }
    Ok(())
}
pub(crate) fn init() -> Result<(), PieFail> {
    if RESOURCES.get().is_some() { return Ok(()) }
    let resources = Resources {
        build: NoleMeta::try_new(TaskId::new(0)).map_err(|_| PieFail::OoM)?,
        doom: NoleMeta::try_new(TaskId::new(0)).map_err(|_| PieFail::OoM)?,
    };
    assert!(RESOURCES.set(resources).is_ok(), "call resources already initialized");
    Ok(())
}
pub(crate) fn get() -> &'static Resources {
    RESOURCES.get().expect("call resources not initialized")
}
