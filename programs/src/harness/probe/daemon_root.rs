pub use system_server::test_support::daemon_scenario::{Mode, Scene};
pub(crate) use system_server::test_support::daemon_scenario::Request;

pub(crate) fn run(settings: Scene) -> Result<(), env::Reason> {
    let boot = system_server::boot::take(router_api::PLIC_CLASS).map_err(|why| why.code())?;
    crate::debug::put(&alloc::format!(
        "{}: root task={}",
        settings.root,
        env::unit::self_id().get(),
    ));
    let deployment = crate::harness::deployment(&boot.catalog).map_err(|_| 9usize)?;
    let declarations = crate::harness::declaration_slice();
    system_server::test_support::daemon_scenario::run(boot, declarations, deployment, settings)
}
