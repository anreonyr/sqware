use super::namespace::EndpointState;
use ::schedule::Resources;
pub(crate) fn resources(
    resources: &mut Resources<'static>,
) -> Result<(), ::schedule::resource::AccessError> {
    resources
        .insert(crate::system::loader::Loader::new())?
        .insert(EndpointState::new())?;
    Ok(())
}
