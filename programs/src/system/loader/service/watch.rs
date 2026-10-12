use super::namespace::EndpointState;
use crate::system::app::Fault as Fail;
use crate::system::app::wait::Interests;
use ::schedule::{Progress, Res, ResMut};
pub(crate) fn entries(
    endpoints: Res<EndpointState>,
    mut wanted: ResMut<Interests>,
) -> Result<Progress, Fail> {
    wanted.tokens.extend(endpoints.inbox.entry);
    Ok(Progress::Done)
}
