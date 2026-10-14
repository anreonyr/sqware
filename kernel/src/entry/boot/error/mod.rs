use ::core::fmt::{self, Write};

use crate::machine::console::Sink;
use crate::machine::hart::HartId;
use crate::core::memory::allocator::InitError;
use crate::core::memory::manager::{MapError, mode::SatpError};
use crate::machine::board::MachineError;
use crate::machine::clock::ClockError;
use crate::entry::report::halt;
use crate::core::work::record::trace::TraceInitError;

mod map_operation;
mod resource_operation;

pub use map_operation::MapOperation;
pub use resource_operation::ResourceOperation;

const TITLE: &str = "sqware: boot failed";

#[derive(Debug)]
pub enum BootError {
    Machine {
        dtp: usize,
        source: MachineError,
    },
    Allocator(erra::Error<InitError>),
    PagingMode(SatpError),
    KernelSpace(erra::Error<MapError>),
    Clock(ClockError),
    Trace(TraceInitError),
    Scheduler(MapError),
    TrapStacks(MapError),
    #[cfg(debug_assertions)]
    Dependencies(crate::machine::sync::DepInitError),
    MissingImage,
    InvalidEntryImage {
        base: usize,
        size: usize,
    },
    Mapping {
        operation: MapOperation,
        source: MapError,
    },
    Resources {
        operation: ResourceOperation,
        source: env::PieFail,
    },
    Bootstrap(MapError),
    BootstrapRelease {
        task: env::TaskId,
        source: env::UnitFail,
    },
    HartStart {
        hart: HartId,
        source: erra::Error<sbi::ecall::SError>,
    },
}

impl BootError {
    fn fields(&self, out: &mut impl Write) -> fmt::Result {
        macro_rules! field {
            ($key:literal, $($value:tt)*) => {
                writeln!(out, "  {:<10} {}", $key, format_args!($($value)*))?
            };
        }
        macro_rules! reason {
            ($operation:literal, $source:expr) => {{
                field!("operation", $operation);
                field!("reason", "{}", $source);
            }};
        }
        match self {
            Self::Machine { dtp, source } => {
                field!("operation", "read machine information");
                field!("dtb", "{dtp:#x}");
                field!("reason", "{source}");
            }
            Self::Allocator(source) => {
                field!("operation", "initialize allocator");
                field!("context", "{}", source.context());
                field!("reason", "{}", source.source);
            }
            Self::PagingMode(source) => reason!("detect page table mode", source),
            Self::KernelSpace(source) => {
                field!("operation", "initialize kernel space");
                field!("context", "{}", source.context());
                field!("reason", "{}", source.source);
            }
            Self::Clock(source) => reason!("initialize clock", source),
            Self::Trace(source) => reason!("initialize trace", source),
            Self::Scheduler(source) => reason!("initialize scheduler", source),
            Self::TrapStacks(source) => reason!("initialize trap stacks", source),
            #[cfg(debug_assertions)]
            Self::Dependencies(source) => {
                field!("operation", "initialize lock dependencies");
                field!(
                    "reason",
                    "{}",
                    match source {
                        crate::machine::sync::DepInitError::OutOfMemory => "out of memory",
                        crate::machine::sync::DepInitError::AlreadyInit => "already initialized",
                    }
                );
            }
            Self::MissingImage => {
                field!("operation", "load boot image");
                field!("reason", "test scene requires an initrd");
            }
            Self::InvalidEntryImage { base, size } => {
                field!("operation", "read boot entry");
                field!("initrd", "{base:#x}");
                field!("size", "{size} bytes");
                field!("reason", "invalid entry descriptor or capsule range");
            }
            Self::Mapping { operation, source } => {
                field!("operation", "{}", operation.label());
                field!("reason", "{source}");
            }
            Self::Resources { operation, source } => {
                field!("operation", "{}", operation.label());
                field!("reason", "{source}");
            }
            Self::Bootstrap(source) => reason!("create bootstrap task", source),
            Self::BootstrapRelease { task, source } => {
                field!("operation", "release bootstrap task");
                field!("task", "{}", task.get());
                field!("reason", "{source}");
            }
            Self::HartStart { hart, source } => {
                field!("operation", "start hart");
                field!("hart", "{hart}");
                field!("context", "{}", source.context());
                field!("reason", "{}", source.source);
            }
        }
        Ok(())
    }

    fn print(&self, sink: &mut impl Write) {
        let _ = writeln!(sink, "{TITLE}\n");
        let _ = self.fields(sink);
        let _ = writeln!(sink);
    }
}

pub fn fail(error: BootError) -> ! {
    halt::stop_boot();
    error.print(&mut Sink);
    if crate::testing() {
        semihosting::process::abort();
    }
    let _ = sbi::SystemResetCall::new(sbi::fid::SystemReset::SystemReset)
        .args(sbi::ecall::SArgs {
            a0: 0,
            a1: 1,
            ..Default::default()
        })
        .call();
    halt::halt_loop()
}

#[cfg(debug_assertions)]
#[path = "../../../../tests/health/boot_errors.rs"]
mod tests;
#[cfg(debug_assertions)]
pub use tests::accept;
