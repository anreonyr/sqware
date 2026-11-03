#![no_std]
#![no_main]
extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    let boot = match system_server::boot::take(router_api::PLIC_CLASS) {
        Ok(boot) => boot,
        Err(error) => {
            return programs::Report::note(error.code(), "system: failed");
        }
    };
    let deployment = match unit::select_deployment(
        programs::composition::DEPLOYMENTS,
        programs::composition::PROGRAMS,
        boot.catalog.deployment(),
        boot.catalog.entry(),
    ) {
        Ok(deployment) => deployment,
        Err(why) => {
            programs::debug!(
                "system: invalid deployment {}: {why:?}",
                boot.catalog.deployment()
            );
            return programs::Report::note(system_server::E_PROGRAM, "system: failed");
        }
    };
    match system_server::run(boot, programs::composition::PROGRAMS, deployment) {
        Ok(()) => programs::Report::note(env::EXIT_OK, "system: done"),
        Err(reason) => programs::Report::note(reason, "system: failed"),
    }
}
