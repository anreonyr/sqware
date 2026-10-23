use crate::{PieToken, Wait};
use crate::system_api::operator::{Entry, Permit};

use super::BoundResource;

pub struct Capability;

impl Capability {
    pub fn find(
        &self,
        _: Entry,
        _: &str,
        _: Permit,
        _: Wait,
    ) -> Result<BoundResource, ()> {
        crate::backend::discover().map(|()| BoundResource {
            resource: PieToken::NONE,
        })
    }
}
