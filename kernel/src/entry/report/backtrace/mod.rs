use crate::core::memory::manager::SpaceKind;

mod config;
mod frame;
mod reader;
mod walk;

pub use config::WalkConfig;
pub use frame::Frame;
pub(super) use frame::FrameKind;
pub use reader::StackReader;
use walk::Walk;

pub const SPAN: usize = 4096;
pub const DEPTH: usize = 32;
const ALIGN: usize = 4;

#[derive(Debug)]
pub struct Backtrace {
    frames: [Frame; DEPTH],
    count: usize,
}

impl Backtrace {
    fn new(world: SpaceKind) -> Self {
        Self {
            frames: [Frame {
                pc: crate::core::memory::manager::addr::VirtAddr::from_raw(0),
                sp: crate::core::memory::manager::addr::VirtAddr::from_raw(0),
                fp: None,
                space: world,
            }; DEPTH],
            count: 0,
        }
    }

    pub(crate) fn frames(&self) -> &[Frame] {
        &self.frames[..self.count]
    }

    fn push(&mut self, frame: Frame) -> bool {
        if self.full()
            || frame.pc.as_usize() == 0
            || frame.pc.as_usize() & (ALIGN - 1) != 0
            || self.frames().last().is_some_and(|last| last.pc == frame.pc)
        {
            return false;
        }
        self.frames[self.count] = frame;
        self.count += 1;
        true
    }

    fn full(&self) -> bool {
        self.count == DEPTH
    }

    pub fn walk(
        reader: &mut StackReader,
        cfg: &WalkConfig,
        sp: usize,
        fp: usize,
        code: Option<&dyn Fn(usize) -> bool>,
    ) -> Self {
        let world = cfg.world;
        let mut walk = Walk::new(reader, fp, world);
        let ceiling = cfg.ceiling;
        let broke = walk.chain(world, sp.saturating_add(16), ceiling);
        if let Some(code) = code {
            walk.scan(code, cfg.gaps, broke.saturating_add(8), ceiling, world);
        }
        walk.finish()
    }
}
