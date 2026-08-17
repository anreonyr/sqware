// 程序装载 — 把程序映像装进地址空间的 durable（常数映射）侧。
//
// 现状输入：手写机器码 blob（≤ 1 页，`&'static [u8]` 静态于内核镜像 .rodata）。
// 三步：① buddy 帧分配 ② blob 字节拷入帧（此刻代码已在 RAM）③ 映射到
// USER_TEXT_BASE（V|R|X|U，帧所有权交给 Space，随 Space drop 归还）。
//
// 阶段 C ELF 加载 = 同一机制推广：解析 PT_LOAD 段 → 逐段 ① ② ③；loader
// 只碰 durable（静态段），栈/帧/堆窗口（dynamic）由 TaskBuilder 分配，正交。

use alloc::boxed::Box;
use alloc::vec;

use crate::memory::PAGE_SIZE;
use crate::memory::allocator::frame::allocator;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::PhysAddr;
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::space::{MapKind, Space};
use crate::work::USER_TEXT_BASE;

/// 装载程序到 Space：分配物理帧 → 拷贝 blob → 映射进 durable（文本段）。
///
/// # Errors
///
/// 帧耗尽（OutOfMemory）或映射冲突（AlreadyMapped / 重叠）。
pub fn load(space: &Space, program: &'static [u8]) -> Result<(), MapError> {
    assert!(program.len() <= PAGE_SIZE, "task program exceeds one page");

    // ① 分配物理帧（buddy；恒等映射下 Box 指针即物理地址）
    let mut text =
        Box::try_new_in([0u8; PAGE_SIZE], allocator()).map_err(|_| MapError::OutOfMemory)?;
    // ② 程序字节从内核镜像拷入帧
    text[..program.len()].copy_from_slice(program);
    let text_pa = PhysAddr::from_raw(text.as_ptr() as usize);
    // ③ 映射进 durable（V|R|X|U，用户态可执行）；帧所有权交 Space
    space.map(
        USER_TEXT_BASE,
        text_pa,
        PAGE_SIZE,
        PteFlags::V | PteFlags::R | PteFlags::X | PteFlags::U | PteFlags::A | PteFlags::D,
        MapKind::Anonymous,
        vec![text],
    )
}
