use super::answer::Inbox;
use ::schedule::Resources;
pub(crate) fn resources(resources: &mut Resources<'static>) -> Result<(), ::schedule::resource::AccessError> {
    resources
        .insert(crate::system::loader::Loader::new())?
        .insert(Inbox::new())?;
    Ok(())
}
