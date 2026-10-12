use crate::{PieToken, Wait};
use system_api::operator::{Entry, EntryId, EntryTarget, NamespaceKey, Path};

#[derive(Clone, Copy)]
pub struct Face { pub entry: Option<PieToken>, pub road: &'static str }
pub struct Tile(PieToken);

const IDENTITY_ROOT: Entry = Entry {
    operator: crate::TaskId::new(1),
    target: EntryTarget::Node(EntryId::new(0)),
};

impl Face {
    pub fn resolve(&self, key: NamespaceKey, _: Wait) -> Result<Entry, ()> {
        (key == system_api::namespace::IDENTITY).then_some(IDENTITY_ROOT).ok_or(())
    }
    pub fn tile_from(&self, base: Entry, road: &Path, _: Wait) -> Result<Tile, ()> {
        if base != IDENTITY_ROOT || road.as_str() != self.road {
            return Err(());
        }
        self.entry.map(Tile).ok_or(())
    }
}
impl Tile {
    pub fn token(&self, _: Wait) -> Result<PieToken, ()> { Ok(self.0) }
}
