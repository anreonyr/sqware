use super::{BootError, ResourceOperation};
use crate::core::memory::flat;
use crate::core::work::{record::trace, room::interrupt};
use crate::entry::call;
use crate::machine::{board, clock, console};

pub fn init(dtp: usize) -> Result<(), BootError> {
    console::init();
    board::init(dtp).map_err(|source| BootError::Machine { dtp, source })?;
    flat::allocator::init().map_err(BootError::Allocator)?;
    // 锁序检测要在任何被验证的调用之前启用：分配器就绪即可建池，
    // 之后的 mode / 空间 / 时钟 / 栈初始化都在它的看护下跑。
    // 池按机器建一次：测试台架会重复走公共 init，重复就是“已经启用”。
    #[cfg(debug_assertions)]
    match crate::machine::sync::init_depend(crate::machine::hart::hart_count()) {
        Ok(()) | Err(crate::machine::sync::DepInitError::AlreadyInit) => {}
        Err(error) => return Err(BootError::Dependencies(error)),
    }
    crate::machine::mode::detect().map_err(BootError::PagingMode)?;
    super::unit::init().map_err(BootError::KernelSpace)?;
    clock::init().map_err(BootError::Clock)?;
    #[cfg(not(feature = "semihosting"))]
    trace::init().map_err(BootError::Trace)?;
    #[cfg(feature = "semihosting")]
    trace::init(crate::entry::report::export::note_sink).map_err(BootError::Trace)?;
    interrupt::init().map_err(|source| BootError::Resources {
        operation: ResourceOperation::InitializeTraps,
        source,
    })?;
    call::resources::init().map_err(|source| BootError::Resources {
        operation: ResourceOperation::InitializeCalls,
        source,
    })?;
    super::stack::init().map_err(BootError::TrapStacks)?;
    Ok(())
}
