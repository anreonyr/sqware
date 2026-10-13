// `VirtAddr::from_raw` 只问页几何要一个 `split_bit`，故这里给一份最小 Sv39 口径。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Sv39,
}

#[derive(Clone, Copy, Debug)]
pub struct Geo {
    pub levels: u8,
    pub va_bits: u8,
}

impl Geo {
    pub fn split_bit(self) -> u8 {
        self.va_bits - 1
    }
}

pub fn mode() -> Mode {
    Mode::Sv39
}

pub fn geometry(_: Mode) -> Geo {
    Geo {
        levels: 3,
        va_bits: 39,
    }
}
