use crate::unit::UnitFile;

pub static PROGRAM: UnitFile = UnitFile {
    name: "iommu-ruin",
    loader: crate::unit::LoaderConfig::supervisor("iommu-ruin"),
    operator: crate::unit::OperatorConfig { namespace: &[] },
    identity: crate::unit::Identity::DEFAULT,
    control: crate::unit::ControlConfig::DEFAULT,
    supply: crate::unit::SupplyConfig::DEFAULT,
};
