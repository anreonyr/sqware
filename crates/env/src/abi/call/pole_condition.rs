use super::pole::Bit;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PoleCondition {
    Signal(Bit),
}

impl crate::Wire for PoleCondition {
    fn pack(&self, words: &mut [usize; 6], at: &mut usize) {
        let Self::Signal(bit) = self;
        (3 + bit.index()).pack(words, at);
    }

    fn unpack(words: &[usize; 6], at: &mut usize) -> Result<Self, crate::Decode> {
        let raw = usize::unpack(words, at)?;
        raw.checked_sub(3)
            .and_then(Bit::of)
            .map(Self::Signal)
            .ok_or(crate::Decode::Invalid)
    }
}
