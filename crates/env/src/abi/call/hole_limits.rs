use crate::{Decode, Wire};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HoleLimits {
    pub max_len: usize,
    pub max_messages: usize,
    pub max_bytes: usize,
}

impl Default for HoleLimits {
    fn default() -> Self {
        Self {
            max_len: isize::MAX as usize,
            max_messages: 4,
            max_bytes: usize::MAX,
        }
    }
}

impl HoleLimits {
    pub fn valid(self) -> bool {
        self.max_len > 0
            && self.max_len <= isize::MAX as usize
            && self.max_messages > 0
            && self.max_bytes >= self.max_len
    }
}

impl Wire for HoleLimits {
    fn pack(&self, words: &mut [usize; 6], at: &mut usize) {
        self.max_len.pack(words, at);
        self.max_messages.pack(words, at);
        self.max_bytes.pack(words, at);
    }

    fn unpack(words: &[usize; 6], at: &mut usize) -> Result<Self, Decode> {
        let limits = Self {
            max_len: usize::unpack(words, at)?,
            max_messages: usize::unpack(words, at)?,
            max_bytes: usize::unpack(words, at)?,
        };
        if limits.valid() {
            Ok(limits)
        } else {
            Err(Decode::Invalid)
        }
    }
}
