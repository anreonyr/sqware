use mold::Fail;

#[derive(Fail)]
pub enum NoleFail {
    Denied = -1,
    Dead = -2,
    #[busy]
    Busy = -3,
    OoM = -4,
    HandedOver = -5,
}
