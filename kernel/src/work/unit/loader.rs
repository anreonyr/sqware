// 程序装载 — 把已解析的 ELF 段装进地址空间的 durable（常数映射）侧。
//
// 只碰 durable（静态段）与堆窗口装载期注册；栈/帧窗口由任务构建时分配，正交。
// 装载整体为一个 `Space::with_flush` 事务：单临界区 + 单次 TLB 刷新，任一段失败
// 则整体不落（原子装载）。

use alloc::boxed::Box;
use alloc::vec::Vec;

use super::parser::{LoadSegment, ParsedProgram};
use super::space::window::HeapWindow;
use super::space::{MapKind, Space};
use crate::layout::STACK_WINDOW_SIZE;
use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::mode;
use crate::memory::manager::table::Frame;

/// 装载产物 — 已装完的空间 + 绝对入口。
pub struct Loaded {
    pub space: Space,
    pub entry: VirtAddr,
}

/// 装载结果 — 复用现成映射错误域（load 全走 `Space` 事务入口，不新造错误类型）。
pub type LoadResult<T> = Result<T, MapError>;

/// 按设计契约装载：把 parsed 的每段映射进 space（文件实体拷帧 + BSS 尾段懒登记），
/// 产出携空间与入口的 Loaded。
///
/// - space **按值**进入：装载本质 = 映射进这块空间，空间归 loader 持有。
/// - 段按 parser 校验后的终态（vaddr/offset 页对齐、X⊓W=∅ 已由 parse 保证）。
///
/// # Errors
///
/// 帧耗尽（OutOfMemory）或映射冲突（AlreadyMapped / NotAligned，均不改动 space）。
pub fn load(space: Space, bytes: &[u8], parsed: &ParsedProgram) -> LoadResult<Loaded> {
    // 0. 装载期注册堆窗口 + 逐段装配——单个 with_flush 临界区：一次加锁、
    //    一次 TLB 刷新；任一段失败 → 整体不落（原子装载）。
    //    堆几何随映像（不魔数），须先于文件段映射注册。
    let image_end = parsed
        .segments
        .iter()
        .map(|s| (s.vaddr.as_usize() + s.memsz).next_multiple_of(PAGE_SIZE))
        .max()
        .unwrap_or(0);
    space.with_flush(|inner| {
        // 0. 堆窗口 `[image_end, 栈底)`：注册只动簿记（校验 + 重叠 + push）
        let edge = mode::upper().as_usize() - STACK_WINDOW_SIZE;
        if !image_end.is_multiple_of(PAGE_SIZE) || image_end > edge {
            return Err(MapError::NotAligned);
        }
        if inner.heap.is_some() {
            return Err(MapError::AlreadyMapped);
        }
        if inner.overlaps(VirtAddr::from_raw(image_end), edge - image_end) {
            return Err(MapError::AlreadyMapped);
        }
        inner.heap = Some(HeapWindow::new(image_end, edge));
        // 1. 逐段：拷帧（文件实体）+ 常数侧装配（权限 = parser 给出 flags，补 V|A|D|U）
        for seg in &parsed.segments {
            let flags = seg.flags | PteFlags::V | PteFlags::A | PteFlags::D;
            inner
                .attach_durable(seg.vaddr, frames_for_segment(bytes, seg)?, flags, MapKind::Anonymous)?;
        }
        Ok(())
    })?;
    // 入口：绝对 VMA。
    Ok(Loaded {
        space,
        entry: parsed.entry,
    })
}

/// 为段分配帧并拷文件字节（一次性造好帧清单；装配由 `attach_durable` 完成）。
fn frames_for_segment(bytes: &[u8], seg: &LoadSegment) -> Result<Vec<Frame>, MapError> {
    let pages = seg.filesz.div_ceil(PAGE_SIZE);
    let mut frames = Vec::with_capacity(pages);
    for i in 0..pages {
        let mut frame: Frame = unsafe {
            Box::try_new_zeroed_in(crate::memory::allocator::frame::allocator())
                .map_err(|_| MapError::OutOfMemory)?
                .assume_init()
        };
        let src = seg.offset + i * PAGE_SIZE;
        let end = seg.offset.saturating_add(seg.filesz);
        let len = end.min(src.saturating_add(PAGE_SIZE)) - src;
        frame[..len].copy_from_slice(&bytes[src..src + len]);
        frames.push(frame);
    }
    Ok(frames)
}
