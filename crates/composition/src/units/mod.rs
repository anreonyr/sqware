//! Remaining in-repo Unit declarations.

pub use iommu_declaration as iommu;
pub use pci_declaration as pci;
pub use shell_declaration as shell;
use unit::UnitFile;

pub mod accept;
pub mod account;
pub mod again;
pub mod beat;
pub mod busy;
pub mod churn;
pub mod daemon;
pub mod daemon_rtc;
pub mod daemon_rtc_control;
pub mod daemon_stop;
pub mod daemon_stop_control;
pub mod daemon_target;
pub mod daemon_target_control;
pub mod device;
pub mod device_confirmation;
pub mod device_guard;
pub mod device_ruin;
pub mod fixturetarget;
pub mod group;
pub mod guest;
pub mod hang;
pub mod hub;
pub mod iommu_dma;
pub mod iommu_probe;
pub mod load;
pub mod lodger;
pub mod login;
pub mod member;
pub mod memory_transfer;
pub mod memory_transfer_fresh;
pub mod memory_transfer_race;
pub mod memory_transfer_tlb;
pub mod park;
pub mod passer;
pub mod pipe;
pub mod probe_bound;
pub mod probe_coalition;
pub mod probe_control;
pub mod probe_denied;
pub mod probe_lease;
pub mod probe_operator_gate;
pub mod probe_operator_land;
pub mod probe_owner;
pub mod probe_rack;
pub mod probe_rack_guest;
pub mod probe_rack_mount;
pub mod probe_rule;
pub mod probe_rule_other;
pub mod probe_terminal;
pub mod probe_watch;
pub mod probe_watch_after;
pub mod probe_watch_gone;
pub mod rig;
pub mod router;
pub mod rtc;
pub mod sleeper;
pub mod subject;
pub mod system;
pub mod system_fault;
pub mod terminal;
pub mod uart;
pub mod virtio_blk;
pub mod waiter;

/// Unit declarations in source registration order.
#[rustfmt::skip]
pub const PROGRAMS: &[&UnitFile] = &[
    &hub::PROGRAM,
    &daemon::PROGRAM,
    &daemon_rtc::PROGRAM,
    &daemon_rtc_control::PROGRAM,
    &daemon_target::PROGRAM,
    &daemon_target_control::PROGRAM,
    &fixturetarget::PROGRAM,
    &daemon_stop::PROGRAM,
    &daemon_stop_control::PROGRAM,
    &pipe::PROGRAM,
    &account::PROGRAM,
    &terminal::PROGRAM,
    &login::PROGRAM,
    &shell::PROGRAM,
    &probe_terminal::PROGRAM,
    &guest::GUEST,
    &passer::PASSER,
    &lodger::LODGER,
    &router::PROGRAM,
    &uart::PROGRAM,
    &rtc::PROGRAM,
    &sleeper::SLEEPER,
    &subject::SUBJECT,
    &member::MEMBER,
    &system::PROGRAM,
    &accept::PROGRAM,
    &probe_denied::PROBE_DENIED,
    &system_fault::ENTRY,
    &system_fault::FAULT_UNIT,
    &system_fault::DEPENDENT,
    &system_fault::CHILD,
    &iommu::PROGRAM,
    &pci::PROGRAM,
    &virtio_blk::PROGRAM,
    &device::WORK,
    &device::WORK_BEFORE,
    &device::WORK_AFTER,
    &device::CONTROL,
    &device::CONTROL_FAULT,
    &device::CONTROL_BEFORE,
    &device::CONTROL_AFTER,
    &device::CONTROL_RUIN,
    &device::CONTROL_FORCED,
    &device::CONTROL_GUARD,
    &device::CONTROL_SOAK,
    &device::CONTROL_SOAK_LONG,
    &device::CONTROL_DUAL,
    &device::CONTROL_DUAL_LONG,
    &device::CONTROL_CONFIRMATION,
    &device_ruin::PROGRAM,
    &device_guard::PROGRAM,
    &device_confirmation::PROGRAM,
    &memory_transfer::ENTRY,
    &memory_transfer::UNIT,
    &memory_transfer_tlb::PROGRAM,
    &memory_transfer_fresh::PROGRAM,
    &memory_transfer_race::ENTRY,
    &memory_transfer_race::UNIT,
    &iommu_dma::PROGRAM,
    &iommu_probe::PROGRAM,
    &probe_owner::PROBE_OWNER,
    &probe_rule::PROBE_RULE,
    &probe_rule_other::PROBE_RULE_OTHER,
    &probe_watch::PROBE_WATCH,
    &probe_watch_after::PROBE_WATCH_AFTER,
    &probe_watch_gone::PROBE_WATCH_GONE,
    &probe_lease::PROBE_LEASE,
    &probe_bound::PROBE_BOUND,
    &probe_rack::PROBE_RACK,
    &probe_rack_mount::PROBE_RACK_MOUNT,
    &probe_rack_guest::PROBE_RACK_GUEST,
    &probe_coalition::PROBE_COALITION,
    &probe_control::PROBE_CONTROL,
    &probe_operator_gate::PROBE_OPERATOR_GATE,
    &probe_operator_land::PROBE_OPERATOR_LAND,
    &churn::CHURN,
    &rig::RIG,
    &busy::BUSY,
    &park::PARK,
    &hang::HANG,
    &load::LOAD,
    &beat::BEAT,
    &again::AGAIN,
    &waiter::WAITER,
    &group::GROUP,
];

/// Registry and manifest upper bounds must agree.
const _: () = assert!(PROGRAMS.len() <= env::ledger::manifest::MAX_PROGRAMS);
