//! Source entry for the provider-owned device-ruin scenario.
pub use system_server::test_support::device_ruin::REQUEST;

pub fn run() -> Result<(), env::Reason> {
    let boot = system_server::boot::take(router_api::PLIC_CLASS).map_err(|why| why.code())?;
    let deployment = crate::harness::deployment(&boot.catalog).map_err(|_| 9usize)?;
    let declarations = crate::harness::declaration_slice();
    system_server::test_support::device_ruin::run(boot, declarations, deployment)
}
