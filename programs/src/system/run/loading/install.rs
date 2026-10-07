use super::answer::Inbox;
use ::schedule::Resources;
pub fn resources(resources: &mut Resources<'static>) -> Result<(), &'static str> {
    resources
        .insert(Inbox::new())
        .map_err(|_| "system resource capacity")
}
