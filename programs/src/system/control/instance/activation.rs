use crate::system::control::unit::verdict::Fail;

pub(crate) fn bounded_ready(
    ready: Result<bool, Fail>,
    now: u64,
    deadline: u64,
) -> Result<bool, Fail> {
    match ready {
        Ok(true) => Ok(true),
        Ok(false) if now < deadline => Ok(false),
        Ok(false) => Err(Fail::NotReady),
        Err(fail) => Err(fail),
    }
}
