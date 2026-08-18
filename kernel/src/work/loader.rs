// 程序装载 — 把已解析的 ELF 段装进地址空间的 durable（常数映射）侧。
//
// parser → loader → TaskBuilder 链的中间环：parser（纯）产配方 ParsedProgram，
// 本模块（有副作用）消费配方 + 源字节，把程序装进一个按值持有的 Space，
// 携 Space 与入口交还（Loaded → TeamBuilder）。
//
// load 为设计契约；blob 装载已随 demo 全 ELF 化移除。
// 只碰 durable（静态段）；栈/帧/堆窗口（dynamic）由 TaskBuilder 分配，正交。

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::memory::PAGE_SIZE;
use crate::memory::allocator::frame::allocator;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::space::{MapKind, Space};
use crate::work::parser::{LoadSegment, ParsedProgram};

/// 装载产物 — 已装完的空间 + 绝对入口（交 Team，并供 TaskBuilder 填 sepc）。
pub struct Loaded {
    pub space: Space,
    pub entry: VirtAddr,
}

/// 装载结果 — 复用现成映射错误域（load 全走 Space::map，不新造错误类型）。
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
pub fn load(
    space: Space,
    bytes: &[u8],
    parsed: &ParsedProgram,
) -> LoadResult<Loaded> {
    for seg in &parsed.segments {
        map_segment(&space, bytes, seg)?;
    }
    // 入口：ET_EXEC 为绝对 VMA；PIE（pie=true）加基址留待后续（当前 demo 均静态）。
    Ok(Loaded {
        space,
        entry: parsed.entry,
    })
}

/// 映射单个段：分配帧 + 拷文件字节 + `Space::map`（权限 = parser 给出 flags，
/// 已无 X⊓W；补 V|A|D|U）。BSS 尾段（memsz>filesz）的懒登记本切片暂不处理
/// （当前 demo 段 filesz==memsz）。
fn map_segment(space: &Space, bytes: &[u8], seg: &LoadSegment) -> Result<(), MapError> {
    let pages = seg.filesz.div_ceil(PAGE_SIZE);
    let mut frames = Vec::with_capacity(pages);
    for i in 0..pages {
        let mut frame = Box::try_new_in([0u8; PAGE_SIZE], allocator())
            .map_err(|_| MapError::OutOfMemory)?;
        let src = seg.offset + i * PAGE_SIZE;
        let end = seg.offset.saturating_add(seg.filesz);
        let len = end.min(src.saturating_add(PAGE_SIZE)) - src;
        frame[..len].copy_from_slice(&bytes[src..src + len]);
        frames.push(frame);
    }
    let pa = PhysAddr::from_raw(frames[0].as_ptr() as usize);
    let flags = seg.flags | PteFlags::V | PteFlags::A | PteFlags::D;
    space.map(
        seg.vaddr,
        pa,
        pages * PAGE_SIZE,
        flags,
        MapKind::Anonymous,
        frames,
    )
}
