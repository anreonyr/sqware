pub use system_server::test_support::device_confirmation_scenario::{ACK, REQUEST};

pub fn run() -> Result<(), env::Reason> {
    let boot = system_server::boot::take(router_api::PLIC_CLASS).map_err(|why| why.code())?;
    let primitive = boot.catalog.deployment() == "iommu-fail-reference";
    let before_stop = boot.catalog.deployment() == "iommu-fail-reset";
    let (party, op, target) = match boot.catalog.deployment() {
        "iommu-fail-invalidate" => (
            hub_api::PARTY_IOMMU,
            crate::driver::iommu::api::INVALIDATE,
            "iommu",
        ),
        "iommu-fail-reset" => (
            hub_api::PARTY_PCI,
            crate::driver::pci::api::DISABLE,
            "pci",
        ),
        "iommu-fail-release" | "iommu-fail-reference" => (
            hub_api::PARTY_IOMMU,
            crate::driver::iommu::api::RELEASE,
            "iommu",
        ),
        _ => return Err(9),
    };
    let deployment = crate::harness::deployment(&boot.catalog).map_err(|_| 9usize)?;
    let declarations = crate::harness::declaration_slice();
    system_server::test_support::device_confirmation_scenario::run(
        boot,
        declarations,
        deployment,
        system_server::test_support::ConfirmationConfig {
            party,
            op,
            target,
            primitive,
            before_stop,
        },
    )
}
