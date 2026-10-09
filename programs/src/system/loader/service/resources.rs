use super::answer::Inbox;
use ::schedule::Resources;
pub(crate) fn resources(resources: &mut Resources<'static>) -> Result<(), &'static str> {
    resources.insert(crate::system::loader::Loader::new())
        .map_err(|_| "Loader resource capacity")?;
    resources
        .insert(Inbox::new())
        .map_err(|_| "system resource capacity")
}
