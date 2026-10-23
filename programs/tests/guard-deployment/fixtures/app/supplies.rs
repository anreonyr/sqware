pub struct Supplies;

impl Supplies {
    pub fn release_target(&mut self, _: crate::TaskId) -> Result<(), ()> {
        Ok(())
    }
}
