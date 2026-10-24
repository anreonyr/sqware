use super::{Action, Origin};

/// The action together with the immutable authority context that asked for it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Directive {
    pub action: Action,
    pub origin: Origin,
}
