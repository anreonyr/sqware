pub struct HartId;

impl HartId {
    pub fn get(self) -> usize {
        0
    }
}

pub fn hart_id() -> HartId {
    HartId
}
