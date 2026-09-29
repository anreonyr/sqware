use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::scene::Scene;
use crate::memory::manager::addr::VirtAddr;
use crate::runtime::diagnose::frame::Frame;
use crate::work::unit::space::SpaceKind;

fn hex(x: usize) -> String {
    format!("{x:#018x}")
}

pub(crate) fn symbol(va: VirtAddr) -> String {
    format!("{:#x}", va.as_usize())
}

const DEPTH: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    Root,
    Kernel,
    Normal,
    Unknown,
}

#[derive(Debug)]
pub struct Backtrace {
    frames: [Frame; DEPTH],
    count: usize,
}

impl Backtrace {
    pub(crate) fn frames(&self) -> &[Frame] {
        &self.frames[..self.count]
    }

    pub(crate) fn from_walk(r: ([Frame; DEPTH], usize)) -> Backtrace {
        Backtrace {
            frames: r.0,
            count: r.1,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FrameResolver {
    world: SpaceKind,
}

impl FrameResolver {
    fn new(world: SpaceKind) -> FrameResolver {
        FrameResolver { world }
    }

    fn classify(&self, pc: VirtAddr) -> FrameKind {
        let k = crate::layout::kernel_edge();
        if pc.as_usize() >= k && pc.as_usize() < k + crate::layout::ROOT_STACK_SIZE {
            return FrameKind::Root;
        }
        if pc.is_kernel() {
            return FrameKind::Kernel;
        }
        if pc.is_user() {
            return FrameKind::Normal;
        }
        if self.executable(pc) {
            return match self.world {
                SpaceKind::Supervisor => FrameKind::Kernel,
                SpaceKind::User => FrameKind::Normal,
            };
        }
        FrameKind::Unknown
    }

    fn executable(&self, _pc: VirtAddr) -> bool {
        false
    }
}

fn kind_label(k: FrameKind) -> &'static str {
    match k {
        FrameKind::Root => "R",
        FrameKind::Kernel => "K",
        FrameKind::Normal => "N",
        FrameKind::Unknown => "?",
    }
}

pub(crate) fn backtrace_rows(scene: &Scene, head: &str) -> Vec<Vec<Option<String>>> {
    let resolver = FrameResolver::new(scene.space);
    let frames = scene.backtrace.frames();
    let mut rows: Vec<Vec<Option<String>>> = vec![vec![
        Some(head.into()),
        Some("kind".into()),
        Some("pc".into()),
        Some("space".into()),
        Some("sp".into()),
        Some("fp".into()),
    ]];
    for (i, f) in frames.iter().enumerate() {
        let kind = resolver.classify(f.pc);
        rows.push(vec![
            Some(format!("#{i}")),
            Some(kind_label(kind).into()),
            Some(hex(f.pc.as_usize())),
            Some(format!("{:?}", f.space)),
            Some(hex(f.sp.as_usize())),
            Some(
                f.fp.map(|v| hex(v.as_usize()))
                    .unwrap_or_else(|| "-".into()),
            ),
        ]);
    }
    rows
}
