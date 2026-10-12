pub(crate) mod book;
mod service;
pub(crate) use service::run::serve as run;
pub mod export;
pub(crate) mod namespace;
pub(crate) mod revision;

pub(crate) fn install(
    resources: &mut ::schedule::Resources<'static>,
) -> Result<(), crate::system::app::InstallError> {
    use ::resource::bell::Bell;

    let changed = Bell::unseal()?;
    resources
        .insert(namespace::Endpoints::default())?
        .insert(revision::Epoch::new())?
        .insert(revision::Changed(changed))?;
    Ok(())
}

pub(crate) fn maintenance() -> Result<::schedule::Plan<&'static str>, ::schedule::BuildError> {
    let mut plan = ::schedule::Schedule::sequence();
    plan.system("revision.consume", revision::consume)?;
    plan.build()
}
