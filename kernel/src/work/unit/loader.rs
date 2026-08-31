// 程序装载 — 把已解析的 ELF 段装进地址空间（user 段 + 常数映射）。
//
// 装载设置 user 段边界（free_base = image_end）并逐段装配帧；整体为一个
// `Space::with_flush` 事务：单临界区 + 单次 TLB 刷新，任一段失败则整体不落
// （原子装载）。

use alloc::boxed::Box;
use alloc::vec::Vec;

use super::parser::{LoadSegment, ParsedProgram};
use super::space::Space;
use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;
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
    // 装载期设置 user 段 + 逐段装配——单个 with_flush 临界区：一次加锁、一次
    // TLB 刷新；任一段失败 → 整体不落（原子装载）。user 段几何随映像（不魔数）。
    let image_end = parsed
        .segments
        .iter()
        .map(|s| (s.vaddr.as_usize() + s.memsz).next_multiple_of(PAGE_SIZE))
        .max()
        .unwrap_or(0);
    space.with_flush(|inner| {
        if !image_end.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::NotAligned);
        }
        inner.attach_free(image_end);
        for seg in &parsed.segments {
            let flags = seg.flags | PteFlags::V | PteFlags::A | PteFlags::D;
            inner.map_frames(seg.vaddr, frames_for_segment(bytes, seg)?, flags)?;
        }
        Ok(())
    })?;
    Ok(Loaded {
        space,
        entry: parsed.entry,
    })
}

/// 为段分配帧并拷文件字节（一次性造好帧清单；装配由 `map_frames` 完成）。
fn frames_for_segment(bytes: &[u8], seg: &LoadSegment) -> Result<Vec<Frame>, MapError> {
    let pages = seg.filesz.div_ceil(PAGE_SIZE);
    let mut frames = Vec::with_capacity(pages);
    for i in 0..pages {
        // 类别 = Task：装载段帧（owned 数据帧）属任务生命周期——关机归零。
        let mut frame: Frame = crate::tag!(Task, unsafe {
            Box::try_new_zeroed_in(crate::memory::allocator::frame::allocator())
                .map_err(|_| MapError::OutOfMemory)?
                .assume_init()
        });
        let src = seg.offset + i * PAGE_SIZE;
        let end = seg.offset.saturating_add(seg.filesz);
        let len = end.min(src.saturating_add(PAGE_SIZE)) - src;
        frame[..len].copy_from_slice(&bytes[src..src + len]);
        frames.push(frame);
    }
    Ok(frames)
}
