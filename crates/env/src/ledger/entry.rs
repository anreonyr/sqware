use crate::wire::{Field, Span};
use crate::{Name, Page, PieKind, PieToken};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Reserved;
impl Field for Reserved {
    const WIDTH: usize = 7;
    fn store(&self, out: &mut [u8]) {
        out.fill(0);
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        bytes
            .get(..Self::WIDTH)?
            .iter()
            .all(|b| *b == 0)
            .then_some(Self)
    }
}

#[derive(crate::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry {
    name: Name,
    kind: PieKind,
    reserved: Reserved,
    token: PieToken,
}
pub const ENTRY_LEN: usize = Entry::LEN;
const _: () = assert!(ENTRY_LEN == 32);
impl Entry {
    pub const NONE: Self = Self::new(Name::Page(Page::Dtb), PieKind::Pole, PieToken::NONE);
    pub const fn new(name: Name, kind: PieKind, token: PieToken) -> Self {
        Self {
            name,
            kind,
            reserved: Reserved,
            token,
        }
    }
    pub const fn name(&self) -> Name {
        self.name
    }
    pub const fn kind(&self) -> PieKind {
        self.kind
    }
    pub const fn token(&self) -> PieToken {
        self.token
    }
    pub fn valid(&self) -> bool {
        self.token != PieToken::NONE
            && self.kind == self.name.kind()
            && !matches!(self.name, Name::Page(Page::Region(0)))
    }
}
pub const VERSION: u32 = 1;
#[derive(crate::Frame, Clone, Copy, Debug)]
pub struct Header {
    pub version: u32,
    pub count: u32,
}

pub struct Entries<'a> {
    bytes: &'a [u8],
    count: usize,
}
impl<'a> Entries<'a> {
    pub fn new(bytes: &'a [u8]) -> Option<Self> {
        let (header, at) = Header::fetch_at(bytes, 0)?;
        let count = usize::try_from(header.count).ok()?;
        let end = at.checked_add(count.checked_mul(ENTRY_LEN)?)?;
        if header.version != VERSION || end != bytes.len() {
            return None;
        }
        let entries = Self {
            bytes: bytes.get(at..end)?,
            count,
        };
        for i in 0..count {
            let entry = Entry::fetch_at(entries.bytes, i * ENTRY_LEN)?.0;
            if !entry.valid()
                || entries
                    .iter()
                    .take(i)
                    .any(|prior| prior.name() == entry.name())
            {
                return None;
            }
        }
        Some(entries)
    }
    pub fn iter(&self) -> impl Iterator<Item = Entry> + '_ {
        (0..self.count).filter_map(|i| Entry::fetch_at(self.bytes, i * ENTRY_LEN).map(|x| x.0))
    }
    pub fn find(&self, name: Name) -> Option<Entry> {
        self.iter().find(|entry| entry.name() == name)
    }
}
