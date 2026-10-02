use alloc::sync::Arc;

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::work::unit::space::Space;
use crate::work::unit::space::SpaceKind;

pub const SPAN: usize = 4096;
const ALIGN: usize = 4;
pub const DEPTH: usize = 32;

#[derive(Debug, Clone, Copy)]
pub struct Frame {
    pub pc: VirtAddr,
    pub sp: VirtAddr,
    pub fp: Option<VirtAddr>,
    pub space: SpaceKind,
}

#[derive(Debug, Clone, Copy)]
pub struct ResolveCfg {
    pub world: SpaceKind,
    pub gaps: bool,
    pub ceiling: usize,
}

impl ResolveCfg {
    pub fn kernel(ceiling: usize) -> ResolveCfg {
        ResolveCfg {
            world: SpaceKind::Supervisor,
            gaps: false,
            ceiling,
        }
    }

    pub fn normal(world: SpaceKind, ceiling: usize) -> ResolveCfg {
        ResolveCfg {
            world,
            gaps: true,
            ceiling,
        }
    }
}

#[derive(Clone)]
pub struct StackReader {
    space: Option<Arc<Space>>,
    root: PhysAddr,
    page: Option<(usize, PhysAddr)>,
}

impl StackReader {
    pub fn new(root_ppn: usize) -> StackReader {
        StackReader {
            space: None,
            root: PhysAddr::from_raw(root_ppn << 12),
            page: None,
        }
    }

    pub fn user(space: Arc<Space>) -> Self {
        Self {
            space: Some(space),
            root: PhysAddr::from_raw(0),
            page: None,
        }
    }

    fn leaf(&mut self, page: usize) -> Option<PhysAddr> {
        if let Some((cached, base)) = self.page
            && cached == page
        {
            return Some(base);
        }
        let edge = crate::platform::machine::dram_edge().unwrap_or(0x9000_0000);
        let (base, flags) = crate::memory::manager::table::TableNode::walk_raw(
            self.root,
            VirtAddr::from_raw(page),
            |pa| (0x8000_0000..edge).contains(&pa.as_usize()),
        )?;
        if !flags.contains(PteFlags::R) {
            return None;
        }
        self.page = Some((page, base));
        Some(base)
    }

    pub fn word(&mut self, addr: usize) -> Option<usize> {
        if let Some(space) = &self.space {
            let mut bytes = [0u8; size_of::<usize>()];
            return space
                .copy_in(&mut bytes, addr)
                .then(|| usize::from_le_bytes(bytes));
        }
        if (addr & (PAGE_SIZE - 1)) + size_of::<usize>() > PAGE_SIZE {
            return None;
        }
        let page = addr & !(PAGE_SIZE - 1);
        let base = self.leaf(page)?;
        // SAFETY: 该页已 walk 命中且带 R；S 态直读
        Some(unsafe {
            (base.as_usize() as *const u8)
                .add(addr - page)
                .cast::<usize>()
                .read_unaligned()
        })
    }

    pub fn pair(&mut self, frame: usize) -> Option<(usize, usize)> {
        Some((
            self.word(frame.checked_sub(16)?)?,
            self.word(frame.checked_sub(8)?)?,
        ))
    }
}

struct Sift<'a> {
    code: &'a dyn Fn(usize) -> bool,
    gaps: bool,
}

struct Walk<'a> {
    reader: &'a mut StackReader,
    fp: usize,
    frames: [Frame; DEPTH],
    count: usize,
    last: usize,
}

impl<'a> Walk<'a> {
    fn new(reader: &'a mut StackReader, fp: usize, world: SpaceKind) -> Walk<'a> {
        Walk {
            reader,
            fp,
            frames: [Frame {
                pc: VirtAddr::from_raw(0),
                sp: VirtAddr::from_raw(0),
                fp: None,
                space: world,
            }; DEPTH],
            count: 0,
            last: 0,
        }
    }

    fn chain(&mut self, world: SpaceKind, floor: usize, ceiling: usize) -> usize {
        let mut f = self.fp;
        let mut broke = self.fp;
        while !self.full() && f >= floor && f <= ceiling {
            broke = f;
            let Some((caller, ra)) = self.reader.pair(f) else {
                break;
            };
            self.push(Frame {
                pc: VirtAddr::from_raw(ra),
                sp: VirtAddr::from_raw(f),
                fp: (caller != 0 && caller > f).then(|| VirtAddr::from_raw(caller)),
                space: world,
            });
            if caller == 0 || caller <= f {
                break;
            }
            f = caller;
        }
        broke
    }

    fn scan(&mut self, sift: &Sift, from: usize, to: usize, world: SpaceKind) {
        let mut a = from;
        while a < to && !self.full() {
            match self.reader.word(a) {
                Some(w) => {
                    if (sift.code)(w) {
                        self.push(Frame {
                            pc: VirtAddr::from_raw(w),
                            sp: VirtAddr::from_raw(a),
                            fp: None,
                            space: world,
                        });
                    }
                    let Some(next) = a.checked_add(8) else { break };
                    a = next;
                }
                None if sift.gaps => {
                    let Some(next) = (a & !(PAGE_SIZE - 1)).checked_add(PAGE_SIZE) else {
                        break;
                    };
                    a = next;
                }
                None => break,
            }
        }
    }

    fn push(&mut self, f: Frame) -> bool {
        if self.full()
            || f.pc.as_usize() == 0
            || f.pc.as_usize() & (ALIGN - 1) != 0
            || f.pc.as_usize() == self.last
        {
            return false;
        }
        self.frames[self.count] = f;
        self.count += 1;
        self.last = f.pc.as_usize();
        true
    }

    fn full(&self) -> bool {
        self.count == DEPTH
    }
}

pub fn walk(
    reader: &mut StackReader,
    cfg: &ResolveCfg,
    sp: usize,
    fp: usize,
    code: Option<&dyn Fn(usize) -> bool>,
) -> ([Frame; DEPTH], usize) {
    let world = cfg.world;
    let mut w = Walk::new(reader, fp, world);
    let ceiling = cfg.ceiling;
    let broke = w.chain(world, sp.saturating_add(16), ceiling);
    if let Some(code) = code {
        let sift = Sift {
            code,
            gaps: cfg.gaps,
        };
        w.scan(&sift, broke.saturating_add(8), ceiling, world);
    }
    (w.frames, w.count)
}
