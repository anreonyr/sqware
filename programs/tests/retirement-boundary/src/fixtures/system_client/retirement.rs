use crate::Wait;

pub struct Face;

impl Face {
    pub fn of(_: crate::PieToken) -> Result<Self, ()> {
        Ok(Self)
    }

    pub fn begin(
        &self,
        _: Wait,
    ) -> Result<crate::ipc::rpc::request::Pending<crate::system_api::retirement::Call>, ()> {
        crate::backend::begin().map(|()| crate::ipc::rpc::request::Pending::new())
    }
}
