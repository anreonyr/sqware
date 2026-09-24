// 程序装载 — 把已解析的 ELF 段装进地址空间（user 段 + 常数映射）。
//
// **两趟**：趟一在**锁外**逐段造帧、向源取字节；趟二在**一次** `Space::with_flush`
// 临界区里落表（`dynamic` + `attach` / 懒登记）。分开不是口味，是锁纪律的硬要求——
// 源空间的 `inner` 与本空间的 `inner` 同为 `Level::Space`，同层嵌套即违规，而
// `with_flush` 的纪律又写着闭包内不得再调任何 `Space` 方法。
//
// 原子性：趟一失败 ⇒ 本空间**一个 PTE 未落**（已造帧随 `Vec` drop 归还）；趟二失败
// ⇒ 空间交回调用方 drop。两路都"不落脏域"。

use alloc::boxed::Box;
use alloc::vec::Vec;

use super::parser::{LoadSegment, ParsedProgram};
use super::source::Source;
use super::space::{Pending, Space};
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

/// 装载失败域 — **映射失败与"源读不到"是两件事**：后者不是映射错误，也不该被折进
/// `OoM` / `AlreadyMapped` 里的任何一格（那会把"这一页没映射"说成"内存不够"）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// 映射失败（帧耗尽 / 区间冲突 / 未对齐）。
    Map(MapError),
    /// 源读不到：段实体那一段区间（或其中一页）没映射。
    Unreadable,
}

impl From<MapError> for LoadError {
    fn from(e: MapError) -> Self {
        LoadError::Map(e)
    }
}

/// 按设计契约装载：把 parsed 的每段映射进 space（文件实体拷帧 + BSS 尾段懒登记），
/// 产出携空间与入口的 Loaded。
///
/// - space **按值**进入：装载本质 = 映射进这块空间，空间归 loader 持有。
/// - 段按 parser 校验后的终态（vaddr/offset 页对齐、X⊓W=∅、段在文件内不重叠，均已由
///   parse 保证）。
///
/// # Errors
///
/// [`LoadError::Map`]（帧耗尽 / 映射冲突 / 未对齐）或 [`LoadError::Unreadable`]（源
/// 读不到）。**越界不是装载的失败域**：`ParsedProgram` 已验每段 `file_end ≤ file_len`，
/// 故 `Source::read` 不会因越界而答 `false`——剩下的唯一来路是"那几页当时没映射"
/// （本域另一枚线程可以并发 `munmap`，故它不能靠"先验一遍"消掉）。
pub fn load(space: Space, source: &Source, parsed: &ParsedProgram) -> Result<Loaded, LoadError> {
    // 趟一（**锁外**）：逐段造帧并填字节。这一步不能挪进下面的临界区——读源要取源空间
    // 的 `inner`，与本空间的 `inner` 同层。
    let mut plan: Vec<Vec<Frame>> = Vec::new();
    // 簿记分配不得 panic（与 `mail` 的暂存同一条纪律）。
    plan.try_reserve(parsed.segments.len())
        .map_err(|_| LoadError::Map(MapError::OutOfMemory))?;
    for seg in &parsed.segments {
        plan.push(frames_for_segment(source, seg)?);
    }

    // 趟二（**一次**临界区）：落表。user 段几何随映像（不魔数）。
    let image_end = parsed
        .segments
        .iter()
        .map(|s| (s.vaddr.as_usize() + s.memsz).next_multiple_of(PAGE_SIZE))
        .max()
        .unwrap_or(0);
    space.with_flush(|inner| -> Result<(), MapError> {
        if !image_end.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::NotAligned);
        }
        inner.dynamic(image_end);
        for (seg, frames) in parsed.segments.iter().zip(plan) {
            let flags = space.pte_policy(seg.flags | PteFlags::V | PteFlags::A | PteFlags::D);
            let file_pages = seg.filesz.div_ceil(PAGE_SIZE);
            // 纯 .bss 段（filesz = 0）：无文件实体可拷，整段走下面的懒登记。
            // 链接脚本把 .data/.bss 各自成段，故这种段合法且常见。
            if file_pages > 0 {
                inner.attach(seg.vaddr, frames, flags)?;
            }
            // BSS 尾段（filesz 后的整页零区）：懒登记——首访缺页物化零页。
            // mem_pages == file_pages 时无差额（当前 ELF 即此情形）。
            let mem_pages = seg.memsz.div_ceil(PAGE_SIZE);
            if mem_pages > file_pages {
                let bss_va = seg.vaddr + file_pages * PAGE_SIZE;
                let bss_size = (mem_pages - file_pages) * PAGE_SIZE;
                inner.map(bss_va, bss_size, flags, Some(Pending::Lazy))?;
            }
        }
        Ok(())
    })?;

    Ok(Loaded {
        space,
        entry: parsed.entry,
    })
}

/// 为段分配帧并向源取文件字节（一次性造好帧清单；装配由 `attach` 完成）。
///
/// **页尾余量恒为零**（帧先零化、再填前 `filesz` 字节）：`.bss` 的零语义靠它，这条
/// 不变量不许丢。
fn frames_for_segment(source: &Source, seg: &LoadSegment) -> Result<Vec<Frame>, LoadError> {
    let pages = seg.filesz.div_ceil(PAGE_SIZE);
    let mut frames: Vec<Frame> = Vec::new();
    // 同 `plan`：簿记分配不得 panic。
    frames
        .try_reserve(pages)
        .map_err(|_| LoadError::Map(MapError::OutOfMemory))?;
    for i in 0..pages {
        // 种类 = Image：装载段帧（owned 数据帧）——关机归零。
        let mut frame: Frame = crate::tag!(Image, unsafe {
            Box::try_new_zeroed_in(crate::memory::allocator::frame::allocator())
                .map_err(|_| LoadError::Map(MapError::OutOfMemory))?
                .assume_init()
        });
        let at = seg.offset + i * PAGE_SIZE;
        let end = seg.offset.saturating_add(seg.filesz);
        let len = end.min(at.saturating_add(PAGE_SIZE)) - at;
        // `false` 时帧随本函数返回的 `Err` 一起 drop（半截字节不外泄）。
        if !source.read(at, &mut frame[..len]) {
            return Err(LoadError::Unreadable);
        }
        frames.push(frame);
    }
    Ok(frames)
}
