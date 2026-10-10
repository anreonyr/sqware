//! Resource-injected, resumable execution plans.
pub mod graph;
pub mod plan;
pub mod resource;
pub mod system;

pub use graph::{BuildError, Schedule};
pub use plan::{Cursor, Plan};
pub use resource::{Res, ResMut, Resources};
pub use system::{IntoSystem, Progress, RunError};

pub mod dispatch;
pub use dispatch::{Completion, Dispatch, DispatchError, Invocation};

mod compose;
mod sequence;
pub use sequence::Sequence;
