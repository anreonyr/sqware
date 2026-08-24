// 内存管理 — 物理分配器 + 虚拟内存管理。
//
//   allocator — 物理内存分配器（bump → hybrid → frame/block）
//   manager   — 虚拟内存管理（VirtAddr/PhysAddr、随模式页表、地址空间、缺页、ASID）
//
// 页大小常量 PAGE_SIZE 与 PAGE_SHIFT 留在本模块顶层，分配器与 manager 共用。

pub mod allocator;
pub mod manager;

/// 页大小 (4 KiB) — RISC-V 架构常量（自包含，不再依赖内核 platform）。
pub const PAGE_SIZE: usize = 4096;
/// 页偏移位数。
pub const PAGE_SHIFT: usize = 12;
