#[derive(Default)]
pub struct Roster;

impl Roster {
    pub fn authorize(&mut self, _: crate::TaskId) -> Result<u64, &'static str> {
        crate::record("authorize");
        Ok(0)
    }

    pub fn inherit(
        &mut self,
        _: crate::TaskId,
        _: crate::TaskId,
    ) -> Result<u64, &'static str> {
        crate::record("inherit");
        Ok(0)
    }

    pub fn install_id(&self, _: crate::TaskId) -> Option<u64> {
        None
    }

    pub fn retire_install(&mut self, _: u64) -> Result<(), ()> {
        Ok(())
    }
}
