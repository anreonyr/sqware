//! Parameters shared by capability creation and queries.
use crate::{Decode, Mark, Permission, PieKind, PieToken, TaskId, Wire};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnsealArgs {
    Hole { mark: Mark, limits: HoleLimits },
    Pole { size: usize, shared: bool },
    Nole,
    Tole { shared: bool },
}
impl UnsealArgs {
    pub fn hole(mark: Mark) -> Self {
        Self::Hole {
            mark,
            limits: HoleLimits::default(),
        }
    }
}
impl Wire for UnsealArgs {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        let words = match *self {
            Self::Hole { mark, limits } => [
                0,
                mark.get() as usize,
                limits.max_len,
                limits.max_messages,
                limits.max_bytes,
            ],
            Self::Pole { size, shared } => [1, size, shared as usize, 0, 0],
            Self::Nole => [2, 0, 0, 0, 0],
            Self::Tole { shared } => [3, shared as usize, 0, 0, 0],
        };
        for word in words {
            word.pack(s, i);
        }
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        let mut w = [0; 5];
        for word in &mut w {
            *word = usize::unpack(s, i)?;
        }
        match w {
            [0, mark, max_len, max_messages, max_bytes] => {
                let limits = HoleLimits {
                    max_len,
                    max_messages,
                    max_bytes,
                };
                if !limits.valid() {
                    return Err(Decode::Invalid);
                }
                Ok(Self::Hole {
                    mark: Mark::new(mark as u64),
                    limits,
                })
            }
            [1, size, shared @ 0..=1, 0, 0] => Ok(Self::Pole {
                size,
                shared: shared != 0,
            }),
            [2, 0, 0, 0, 0] => Ok(Self::Nole),
            [3, shared @ 0..=1, 0, 0, 0] => Ok(Self::Tole {
                shared: shared != 0,
            }),
            _ => Err(Decode::Invalid),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseMode {
    Revoke,
    Keep,
}
impl Wire for ReleaseMode {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        (*self as usize).pack(s, i);
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        match usize::unpack(s, i)? {
            0 => Ok(Self::Revoke),
            1 => Ok(Self::Keep),
            _ => Err(Decode::Invalid),
        }
    }
}

/// Seven whole words; no Rust struct padding crosses the ABI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PieInfo {
    pub token: PieToken,
    pub kind: PieKind,
    pub permission: Permission,
    pub owner: TaskId,
    pub vestor: TaskId,
    pub mark: Mark,
    pub alive: bool,
}
impl PieInfo {
    pub const WORDS: usize = 7;
    pub const SIZE: usize = Self::WORDS * core::mem::size_of::<usize>();
    pub fn words(self) -> [usize; 7] {
        [
            self.token.get(),
            self.kind as usize,
            self.permission.bits() as usize,
            self.owner.get(),
            self.vestor.get(),
            self.mark.get() as usize,
            self.alive as usize,
        ]
    }
    pub fn from_words(w: [usize; 7]) -> Option<Self> {
        Some(Self {
            token: PieToken::new(w[0]),
            kind: PieKind::of(u8::try_from(w[1]).ok()?)?,
            permission: Permission::from_bits(u32::try_from(w[2]).ok()?)?,
            owner: TaskId::new(w[3]),
            vestor: TaskId::new(w[4]),
            mark: Mark::new(w[5] as u64),
            alive: match w[6] {
                0 => false,
                1 => true,
                _ => return None,
            },
        })
    }
}
