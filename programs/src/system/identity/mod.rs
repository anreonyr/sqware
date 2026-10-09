pub(crate) mod book;
mod service;
pub(crate) use service::run::serve as run;
pub(crate) mod revision;

pub(crate) fn install(resources: &mut ::schedule::Resources<'static>) -> Result<(), &'static str> {
    use ::resource::bell::Bell;

    let changed = Bell::unseal().map_err(|_| "identity change bell")?;
    resources
        .insert(revision::Epoch::new())
        .map_err(|_| "system resource capacity")?;
    resources
        .insert(revision::Changed(changed))
        .map_err(|_| "system resource capacity")?;
    Ok(())
}
