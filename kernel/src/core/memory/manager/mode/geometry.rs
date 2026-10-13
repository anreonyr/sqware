use riscv::register::satp;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geo {
    pub levels: u8,
    pub va_bits: u8,
}

impl Geo {
    #[inline]
    pub fn split_bit(self) -> u8 {
        self.va_bits - 1
    }
}

pub fn geometry(mode: satp::Mode) -> Geo {
    match mode {
        satp::Mode::Sv39 => Geo {
            levels: 3,
            va_bits: 39,
        },
        satp::Mode::Sv48 => Geo {
            levels: 4,
            va_bits: 48,
        },
        satp::Mode::Sv57 => Geo {
            levels: 5,
            va_bits: 57,
        },
        _ => panic!("geometry: unsupported satp mode {mode:?}"),
    }
}
