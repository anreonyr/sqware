use env::{PieToken, Wait};

pub struct Hole;

impl Hole {
    pub fn from_raw(_: PieToken) -> Self {
        Hole
    }

    pub fn pull(&self, _: &mut [u8], _: Wait) -> Result<(usize, PieToken), ()> {
        Err(())
    }
}
