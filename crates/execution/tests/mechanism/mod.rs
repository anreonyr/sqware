//! 生产 memory 机制的同 owner 源码投影，供夹具内核驱动失败路径。

pub const PAGE_SIZE: usize = env::PAGE_SIZE;

#[path = "../../src/memory/mapping_kernel.rs"]
pub mod mapping_kernel;
#[path = "../../src/memory/reference.rs"]
pub mod reference;
#[path = "../../src/memory/reference_kernel.rs"]
pub mod reference_kernel;
#[path = "../../src/memory/view.rs"]
pub mod view;

#[path = "tests.rs"]
mod tests;
