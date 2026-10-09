pub(crate) mod book;
mod service;
pub(crate) use service::run::serve as run;
pub(crate) mod revision;

pub(crate) fn install(resources: &mut ::schedule::Resources<'static>) -> Result<(), crate::system::app::InstallError> {
    use ::resource::bell::Bell;

    let changed = Bell::unseal()?;
    resources
        .insert(revision::Epoch::new())?
        .insert(revision::Changed(changed))?;
    Ok(())
}
