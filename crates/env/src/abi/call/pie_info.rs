use crate::{Mark, Permission, PieKind, PieToken, TaskId};

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
