//! Only an explicit missing-path reply permits another lookup attempt.
use system_api::operator::Fail;
pub(super) enum Failure {
    Missing,
    Stop(Fail),
}
pub(super) fn until<T>(
    mut call: impl FnMut() -> Result<T, Failure>,
    mut again: impl FnMut(usize) -> bool,
) -> Result<T, Fail> {
    let mut backoff = 10;
    loop {
        match call() {
            Ok(value) => return Ok(value),
            Err(Failure::Missing) if again(backoff) => {
                backoff = (backoff * 2).min(100);
            }
            Err(Failure::Missing) => return Err(Fail::Unknown),
            Err(Failure::Stop(fail)) => return Err(fail),
        }
    }
}
