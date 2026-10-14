#[derive(Debug, Clone, Copy)]
pub enum ResourceOperation {
    InitializeTraps,
    InitializeCalls,
    Devices,
    Traps,
    Calls,
    Freeze,
    LedgerSize,
    Grant,
    WriteLedger,
}

impl ResourceOperation {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::InitializeTraps => "initialize trap resources",
            Self::InitializeCalls => "initialize call resources",
            Self::Devices => "register device resources",
            Self::Traps => "register trap resources",
            Self::Calls => "register call resources",
            Self::Freeze => "freeze resources",
            Self::LedgerSize => "size resource ledger",
            Self::Grant => "grant bootstrap resources",
            Self::WriteLedger => "write resource ledger",
        }
    }
}
