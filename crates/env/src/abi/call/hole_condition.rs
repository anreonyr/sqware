#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HoleCondition {
    Pull,
    Push,
    Empty,
    Signal,
}

impl crate::Wire for HoleCondition {
    fn pack(&self, words: &mut [usize; 6], at: &mut usize) {
        (*self as usize).pack(words, at);
    }

    fn unpack(words: &[usize; 6], at: &mut usize) -> Result<Self, crate::Decode> {
        match usize::unpack(words, at)? {
            0 => Ok(Self::Pull),
            1 => Ok(Self::Push),
            2 => Ok(Self::Empty),
            3 => Ok(Self::Signal),
            _ => Err(crate::Decode::Invalid),
        }
    }
}
