use super::{Cursor, Progress, RunError};

pub struct Invocation<K> {
    pub key: K,
    pub cursor: Cursor,
}
pub struct Dispatch<K, E> {
    pub budget: usize,
    pub current: Option<Invocation<K>>,
    pub result: Option<Result<Progress, RunError<E>>>,
}
impl<K, E> Dispatch<K, E> {
    pub fn new() -> Self {
        Self {
            budget: 0,
            current: None,
            result: None,
        }
    }
}
