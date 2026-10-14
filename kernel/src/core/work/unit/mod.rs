pub(crate) mod capsule;
pub(crate) mod copies;
pub(crate) mod join;
pub(crate) mod life;
pub mod space;
pub(crate) mod tally;
pub(crate) mod task;
pub(crate) mod team;

use crate::core::memory::nest::NestError;

pub type NestResult<T> = erra::Result<T, NestError>;
