use alloc::alloc::AllocError;
use alloc::vec::Vec;

#[derive(Debug)]
pub(crate) struct BitmapAllocator {
    base: usize,
    edge: usize,
    unit: usize,
    bits: Vec<u64>,
}

impl BitmapAllocator {
    pub(crate) const fn new(base: usize, edge: usize, unit: usize) -> Self {
        Self {
            base,
            edge,
            unit,
            bits: Vec::new(),
        }
    }

    pub(crate) fn allocate(&mut self, size: usize) -> Result<(usize, usize), AllocError> {
        self.ensure().map_err(|()| AllocError)?;
        let units = size.div_ceil(self.unit).max(1);

        let mut run_start = 0usize;
        let mut run_len = 0usize;
        let mut found = false;
        for i in 0..self.units() {
            if self.bits[i / 64] & (1 << (i % 64)) == 0 {
                if run_len == 0 {
                    run_start = i;
                }
                run_len += 1;
                if run_len >= units {
                    found = true;
                    break;
                }
            } else {
                run_len = 0;
            }
        }
        if !found {
            return Err(AllocError);
        }

        for i in run_start..run_start + units {
            self.bits[i / 64] |= 1 << (i % 64);
        }
        Ok((self.base + run_start * self.unit, units * self.unit))
    }

    pub(crate) fn deallocate(&mut self, addr: usize, size: usize) -> Result<(), AllocError> {
        let sized = self.ensure().is_ok();
        if !sized || addr < self.base || addr + size > self.edge {
            return Err(AllocError);
        }
        if !addr.is_multiple_of(self.unit) || !size.is_multiple_of(self.unit) {
            return Err(AllocError);
        }
        let start = (addr - self.base) / self.unit;
        let units = size / self.unit;

        for i in start..start + units {
            if self.bits[i / 64] & (1 << (i % 64)) == 0 {
                return Err(AllocError);
            }
        }
        for i in start..start + units {
            self.bits[i / 64] &= !(1 << (i % 64));
        }
        Ok(())
    }

    fn units(&self) -> usize {
        (self.edge - self.base) / self.unit
    }

    fn ensure(&mut self) -> Result<(), ()> {
        if self.bits.is_empty() && self.units() > 0 {
            let words = self.units().div_ceil(64);
            self.bits.try_reserve(words).map_err(|_| ())?;
            self.bits.resize(words, 0);
        }
        Ok(())
    }
}
