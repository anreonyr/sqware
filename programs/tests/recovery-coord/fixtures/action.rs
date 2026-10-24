#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    PciPlace,
    PciEnable,
    PciDisable,
    PciRelease,
    CoreAttach,
    CoreStop,
    CoreDetach,
    IommuAcquire,
    IommuPublish,
    IommuInvalidate,
    IommuQuiesce,
    IommuRelease,
}
