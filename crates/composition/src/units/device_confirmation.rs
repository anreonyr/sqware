use crate::unit::UnitFile;

pub static PROGRAM: UnitFile = UnitFile {
    name: "iommu-confirmation",
    loader: crate::unit::LoaderConfig::supervisor("iommu-confirmation"),
    operator: crate::unit::OperatorConfig { namespace: &[] },
    identity: crate::unit::Identity::DEFAULT,
    control: crate::unit::ControlConfig::DEFAULT,
    supply: crate::unit::SupplyConfig::DEFAULT,
};
