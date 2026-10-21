#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Bit(u8);

impl Bit {
    pub const FIRST: Self = Self(0);
    pub const fn wrap(index: usize) -> Self {
        Self(index as u8)
    }
    pub const fn of(index: usize) -> Option<Self> {
        if index < usize::BITS as usize {
            Some(Self(index as u8))
        } else {
            None
        }
    }
    pub const fn index(self) -> usize {
        self.0 as usize
    }
    pub const fn mask(self) -> usize {
        1usize << self.0
    }
}

impl crate::Wire for Bit {
    fn pack(&self, words: &mut [usize; 6], at: &mut usize) {
        self.index().pack(words, at);
    }
    fn unpack(words: &[usize; 6], at: &mut usize) -> Result<Self, crate::Decode> {
        Self::of(usize::unpack(words, at)?).ok_or(crate::Decode::Invalid)
    }
}
