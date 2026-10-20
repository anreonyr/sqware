use crate::{Decode, Wire};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseMode {
    Revoke,
    Keep,
}

impl Wire for ReleaseMode {
    fn pack(&self, words: &mut [usize; 6], at: &mut usize) {
        (*self as usize).pack(words, at);
    }

    fn unpack(words: &[usize; 6], at: &mut usize) -> Result<Self, Decode> {
        match usize::unpack(words, at)? {
            0 => Ok(Self::Revoke),
            1 => Ok(Self::Keep),
            _ => Err(Decode::Invalid),
        }
    }
}
