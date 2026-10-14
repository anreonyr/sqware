use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::VirtAddr;
use crate::core::memory::nest::SpaceKind;

use super::{Backtrace, Frame, StackReader};

pub(super) struct Walk<'a> {
    reader: &'a mut StackReader,
    fp: usize,
    backtrace: Backtrace,
}

impl<'a> Walk<'a> {
    pub(super) fn new(reader: &'a mut StackReader, fp: usize, world: SpaceKind) -> Self {
        Self {
            reader,
            fp,
            backtrace: Backtrace::new(world),
        }
    }

    pub(super) fn chain(&mut self, world: SpaceKind, floor: usize, ceiling: usize) -> usize {
        let mut f = self.fp;
        let mut broke = self.fp;
        while !self.backtrace.full() && f >= floor && f <= ceiling {
            broke = f;
            let Some((caller, ra)) = self.reader.pair(f) else {
                break;
            };
            self.backtrace.push(Frame {
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

    pub(super) fn scan(
        &mut self,
        code: &dyn Fn(usize) -> bool,
        gaps: bool,
        from: usize,
        to: usize,
        world: SpaceKind,
    ) {
        let mut a = from;
        while a < to && !self.backtrace.full() {
            match self.reader.word(a) {
                Some(w) => {
                    if code(w) {
                        self.backtrace.push(Frame {
                            pc: VirtAddr::from_raw(w),
                            sp: VirtAddr::from_raw(a),
                            fp: None,
                            space: world,
                        });
                    }
                    let Some(next) = a.checked_add(8) else { break };
                    a = next;
                }
                None if gaps => {
                    let Some(next) = (a & !(PAGE_SIZE - 1)).checked_add(PAGE_SIZE) else {
                        break;
                    };
                    a = next;
                }
                None => break,
            }
        }
    }

    pub(super) fn finish(self) -> Backtrace {
        self.backtrace
    }
}
