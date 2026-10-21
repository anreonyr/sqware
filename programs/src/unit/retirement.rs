/// A provider program face that must report retirement before this unit is reclaimed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Retirement {
    pub provider: &'static str,
    pub namespace: usize,
    pub face: &'static str,
}

impl Retirement {
    pub fn valid(&self) -> bool {
        system_api::operator::name::valid(self.provider)
            && system_api::operator::name::valid(self.face)
    }
}
