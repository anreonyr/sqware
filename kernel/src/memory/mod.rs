// 内存管理 — 物理分配器 + 虚拟内存管理（自包含）
//
// 两个子模块：
//   allocator — 物理内存分配器（bump → hybrid → frame/block）
//   manager   — 虚拟内存管理（VirtAddr/PhysAddr、Sv39 页表、地址空间、缺页、ASID）
//
// 自包含：不依赖内核 platform/hal/macros。物理内存池区域由调用方在
// `allocator::init(Region, hart_count)` 注入；satp/hart_id 内化于 arch；
// 日志内化于 log；bitflags! 内化于 macros。把本文件夹（连同 src/lock）复制
// 到另一个 RISC-V 内核项目即可独立编译运行。
//
// 页大小常量 PAGE_SIZE 与 PAGE_SHIFT 留在本模块顶层，分配器与 manager 共用。

// 顺序重要：宏先于使用它们的子模块声明（`#[macro_use]` 文本作用域）。
#[macro_use]
mod macros; // bitflags!（entry.rs 用）
#[macro_use]
mod log; // debug!/warn!（frame/block 用）
mod arch;
mod platform;

pub mod allocator;
pub mod manager;

/// 页大小 (4 KiB) — RISC-V 架构常量（自包含，不再依赖内核 platform）。
pub const PAGE_SIZE: usize = 4096;
/// 页偏移位数。
pub const PAGE_SHIFT: usize = 12;
