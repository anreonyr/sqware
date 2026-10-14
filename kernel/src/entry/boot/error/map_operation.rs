#[derive(Debug, Clone, Copy)]
pub enum MapOperation {
    AssembleCapsule,
    ImageView,
    LedgerView,
}

impl MapOperation {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::AssembleCapsule => "assemble boot capsule",
            Self::ImageView => "map boot image",
            Self::LedgerView => "map resource ledger",
        }
    }
}
