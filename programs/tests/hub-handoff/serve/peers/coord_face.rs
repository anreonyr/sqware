use env::{PieToken, Wait};
use hub_api::coordination::{Call, Wire};
use crate::rpc::request::Pending;

pub struct CoordFace;

impl CoordFace {
    pub fn host(&self) -> env::TaskId { env::TaskId::new(3) }

    pub fn begin(&self, _: Wait, request: impl FnOnce(PieToken) -> Wire)
        -> Result<Pending<Call>, &'static str>
    {
        Ok(Pending::coordinate(request(PieToken::mint(99))))
    }
}
