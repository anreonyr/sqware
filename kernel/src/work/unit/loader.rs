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

pub struct Loaded {
    pub space: Space,
    pub entry: VirtAddr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadError {
    Map(MapError),
    Unreadable,
}

impl From<MapError> for LoadError {
    fn from(e: MapError) -> Self {
        LoadError::Map(e)
    }
}

pub fn load(space: Space, source: &Source, parsed: &ParsedProgram) -> Result<Loaded, LoadError> {
    let image_end = parsed.segments.iter().try_fold(0usize, |edge, segment| {
        segment
            .vaddr
            .as_usize()
            .checked_add(segment.memsz)
            .and_then(|end| end.checked_next_multiple_of(PAGE_SIZE))
            .filter(|end| *end <= crate::memory::manager::mode::upper().as_usize())
            .map(|end| edge.max(end))
            .ok_or(MapError::NoRegion)
    })?;
    let mut plan: Vec<Vec<Frame>> = Vec::new();
    plan.try_reserve(parsed.segments.len())
        .map_err(|_| LoadError::Map(MapError::OutOfMemory))?;
    for seg in &parsed.segments {
        plan.push(frames_for_segment(source, seg)?);
    }

    space.with_flush(|inner| -> Result<(), MapError> {
        if !image_end.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::NotAligned);
        }
        inner.dynamic(image_end);
        for (seg, frames) in parsed.segments.iter().zip(plan) {
            let flags = space.pte_policy(seg.flags | PteFlags::V | PteFlags::A | PteFlags::D);
            let file_pages = seg.filesz.div_ceil(PAGE_SIZE);
            if file_pages > 0 {
                inner.attach(seg.vaddr, frames, flags)?;
            }
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

fn frames_for_segment(source: &Source, seg: &LoadSegment) -> Result<Vec<Frame>, LoadError> {
    let pages = seg.filesz.div_ceil(PAGE_SIZE);
    let mut frames: Vec<Frame> = Vec::new();
    frames
        .try_reserve(pages)
        .map_err(|_| LoadError::Map(MapError::OutOfMemory))?;
    for i in 0..pages {
        let mut frame: Frame = crate::tag!(Image, unsafe {
            Box::try_new_zeroed_in(crate::memory::allocator::frame::allocator())
                .map_err(|_| LoadError::Map(MapError::OutOfMemory))?
                .assume_init()
        });
        let at = seg.offset + i * PAGE_SIZE;
        let end = seg.offset.saturating_add(seg.filesz);
        let len = end.min(at.saturating_add(PAGE_SIZE)) - at;
        if !source.read(at, &mut frame[..len]) {
            return Err(LoadError::Unreadable);
        }
        frames.push(frame);
    }
    Ok(frames)
}
