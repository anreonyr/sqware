use crate::{PieToken, Wait};

#[derive(Clone, Copy)]
pub struct Face { pub entry: Option<PieToken> }
pub struct Tile(PieToken);

impl Face {
    pub fn tile(&self, _: &crate::common::path::Path, _: Wait) -> Result<Tile, ()> {
        self.entry.map(Tile).ok_or(())
    }
}
impl Tile {
    pub fn token(&self, _: Wait) -> Result<PieToken, ()> { Ok(self.0) }
}
