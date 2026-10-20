use mold::Fail;

#[derive(Fail)]
pub enum DispatchFail {
    Unknown = -1,
}
