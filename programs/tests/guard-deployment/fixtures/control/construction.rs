pub struct Construction;

impl Construction {
    pub fn retire<F>(&mut self, _: crate::TaskId, f: F) -> Result<(), ()>
    where
        F: FnOnce(u64, u64) -> Result<(), ()>,
    {
        f(0, 0)
    }
}
