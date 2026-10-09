use super::value::RootValue;
use alloc::{string::String, vec::Vec};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ForeignId {
    pub slot: u32,
    pub generation: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallId(pub(crate) u64);
#[derive(Clone, Copy, Debug)]
pub struct Arity {
    pub min: usize,
    pub max: Option<usize>,
}
impl Arity {
    pub const fn fixed(n: usize) -> Self {
        Self {
            min: n,
            max: Some(n),
        }
    }
    pub const fn variadic(min: usize) -> Self {
        Self { min, max: None }
    }
    pub(crate) fn accepts(self, n: usize) -> bool {
        n >= self.min && self.max.is_none_or(|max| n <= max)
    }
}
#[derive(Clone, Debug)]
pub struct HostError(pub String);
pub type NativeResult = Result<RootValue, HostError>;
#[derive(Debug)]
pub struct Call {
    pub id: CallId,
    pub operation: usize,
    pub arguments: Vec<RootValue>,
}
pub(crate) struct Native {
    pub operation: usize,
    pub arity: Arity,
}
