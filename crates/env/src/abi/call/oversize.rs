#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Oversize {
    Keep,
    Discard,
}

impl crate::Wire for Oversize {
    fn pack(&self, words: &mut [usize; 6], at: &mut usize) {
        (*self as usize).pack(words, at);
    }

    fn unpack(words: &[usize; 6], at: &mut usize) -> Result<Self, crate::Decode> {
        match usize::unpack(words, at)? {
            0 => Ok(Self::Keep),
            1 => Ok(Self::Discard),
            _ => Err(crate::Decode::Invalid),
        }
    }
}
