use core::fmt::{self, Write};

use crate::console::Sink;
use crate::hart::HartId;
use crate::memory::allocator::InitError;
use crate::memory::manager::{MapError, mode::SatpError};
use crate::platform::machine::MachineError;
use crate::runtime::chrono::clock::ClockError;
use crate::runtime::diagnose::halt;
use crate::runtime::diagnose::trace::TraceInitError;

const TITLE: &str = "sqware: boot failed";

#[derive(Debug)]
pub enum BootError {
    Machine { dtp: usize, source: MachineError },
    Allocator(erra::Error<InitError>),
    PagingMode(SatpError),
    KernelSpace(erra::Error<MapError>),
    Clock(ClockError),
    Trace(TraceInitError),
    Scheduler(MapError),
    TrapStacks(MapError),
    #[cfg(debug_assertions)]
    Dependencies(crate::lock::DepInitError),
    MissingImage,
    InvalidEntryImage { base: usize, size: usize },
    Mapping { operation: MapOperation, source: MapError },
    Resources { operation: ResourceOperation, source: env::PieFail },
    Bootstrap(MapError),
    BootstrapRelease { task: env::TaskId, source: env::UnitFail },
    HartStart { hart: HartId, source: erra::Error<sbi::ecall::SError> },
}

#[derive(Debug, Clone, Copy)]
pub enum MapOperation {
    AssembleCapsule,
    ImageView,
    LedgerView,
}

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

impl BootError {
    fn fields(&self, out: &mut ConsoleFields<'_, impl Write>) -> fmt::Result {
        macro_rules! field {
            ($key:literal, $($value:tt)*) => {
                out.field($key, format_args!($($value)*))?
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
                field!("reason", "{}", match source {
                    crate::lock::DepInitError::OutOfMemory => "out of memory",
                    crate::lock::DepInitError::AlreadyInit => "already initialized",
                });
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
                field!("operation", "{}", match operation {
                    MapOperation::AssembleCapsule => "assemble boot capsule",
                    MapOperation::ImageView => "map boot image",
                    MapOperation::LedgerView => "map resource ledger",
                });
                field!("reason", "{source}");
            }
            Self::Resources { operation, source } => {
                field!("operation", "{}", match operation {
                    ResourceOperation::InitializeTraps => "initialize trap resources",
                    ResourceOperation::InitializeCalls => "initialize call resources",
                    ResourceOperation::Devices => "register device resources",
                    ResourceOperation::Traps => "register trap resources",
                    ResourceOperation::Calls => "register call resources",
                    ResourceOperation::Freeze => "freeze resources",
                    ResourceOperation::LedgerSize => "size resource ledger",
                    ResourceOperation::Grant => "grant bootstrap resources",
                    ResourceOperation::WriteLedger => "write resource ledger",
                });
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
        let _ = self.fields(&mut ConsoleFields(sink));
        let _ = writeln!(sink);
    }
}

struct ConsoleFields<'a, W>(&'a mut W);

impl<W: Write> ConsoleFields<'_, W> {
    fn field(&mut self, key: &'static str, value: fmt::Arguments<'_>) -> fmt::Result {
        writeln!(self.0, "  {key:<10} {value}")
    }
}

pub fn fail(error: BootError) -> ! {
    halt::stop_boot();
    error.print(&mut Sink);
    if crate::testing() {
        semihosting::process::abort();
    }
    let _ = sbi::SystemResetCall::new(sbi::fid::SystemReset::SystemReset)
        .args(sbi::ecall::SArgs { a0: 0, a1: 1, ..Default::default() })
        .call();
    halt::halt_loop()
}

#[cfg(debug_assertions)]
pub fn accept() {
    struct Buffer { bytes: [u8; 2048], len: usize }
    impl Buffer {
        fn new() -> Self { Self { bytes: [0; 2048], len: 0 } }
        fn text(&self) -> &str { core::str::from_utf8(&self.bytes[..self.len]).unwrap() }
    }
    impl Write for Buffer {
        fn write_str(&mut self, text: &str) -> fmt::Result {
            let end = self.len.checked_add(text.len()).ok_or(fmt::Error)?;
            let dest = self.bytes.get_mut(self.len..end).ok_or(fmt::Error)?;
            dest.copy_from_slice(text.as_bytes());
            self.len = end;
            Ok(())
        }
    }

    let mut output = Buffer::new();
    {
        let _guard = crate::memory::allocator::NoAllocation::enter();
        BootError::Allocator(erra::Error::new("initializing allocator", InitError::OutOfMemory))
            .print(&mut output);
    }
    assert_eq!(output.text(), concat!(
        "sqware: boot failed\n\n",
        "  operation  initialize allocator\n",
        "  context    initializing allocator\n",
        "  reason     memory allocation failed while initializing allocator\n\n",
    ));

    let error = BootError::HartStart {
        hart: HartId::new(2),
        source: erra::Error::new("s-mode environment call", sbi::ecall::SError::Denied),
    };
    let mut output = Buffer::new();
    {
        let _guard = crate::memory::allocator::NoAllocation::enter();
        error.print(&mut output);
    }
    assert_eq!(output.text(), concat!(
        "sqware: boot failed\n\n",
        "  operation  start hart\n",
        "  hart       2\n",
        "  context    s-mode environment call\n",
        "  reason     access denied\n\n",
    ));
}
