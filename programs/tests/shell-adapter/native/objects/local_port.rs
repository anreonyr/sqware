use crate::adapt::ports::Buffer;
use crate::core::JobId;

pub struct LocalPort {
    pub buffer: Buffer,
    pub closed: bool,
    pub bound: Option<JobId>,
}
