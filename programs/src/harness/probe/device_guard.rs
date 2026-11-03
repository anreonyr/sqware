#[path = "../../driver/uart/dev/uart.rs"]
#[allow(dead_code)]
mod uart;

pub use system_server::test_support::device_guard_scenario::REQUEST;

pub fn run() -> Result<(), env::Reason> {
    let boot = system_server::boot::take(router_api::PLIC_CLASS).map_err(|why| why.code())?;
    let target = match boot.catalog.deployment() {
        "iommu-guard-pci" => "pci",
        "iommu-guard-iommu" => "iommu",
        "iommu-guard-virtio-blk" => "virtio-blk",
        _ => return Err(9usize),
    };
    crate::debug::put(&alloc::format!(
        "guard: root task={}", env::unit::self_id().get(),
    ));
    let deployment = crate::harness::deployment(&boot.catalog).map_err(|_| 9usize)?;
    let declarations = crate::harness::declaration_slice();
    system_server::test_support::device_guard_scenario::run(
        boot,
        declarations,
        deployment,
        system_server::test_support::GuardSettings {
            target,
            drain: uart::drain,
        },
    )
}
