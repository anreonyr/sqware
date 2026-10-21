use system_api::retirement::Call;
use system_client::retirement::Face;

/// One declared retirement provider and its in-flight request.
pub(crate) struct Participant {
    pub(crate) declaration: crate::unit::Retirement,
    pub(crate) face: Option<Face>,
    pub(crate) pending: Option<ipc::rpc::request::Pending<Call>>,
    pub(crate) done: bool,
}

impl Participant {
    pub(crate) fn new(declaration: crate::unit::Retirement) -> Self {
        Self {
            declaration,
            face: None,
            pending: None,
            done: false,
        }
    }
}
