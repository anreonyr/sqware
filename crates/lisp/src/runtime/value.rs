use alloc::{collections::BTreeMap, rc::Rc};
use core::cell::RefCell;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Handle {
    pub slot: usize,
    pub generation: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Value {
    Nil,
    Integer(i64),
    Boolean(bool),
    Object(Handle),
    Builtin(usize),
    Native(usize),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
    Nil,
    Integer,
    Boolean,
    Symbol,
    String,
    Bytes,
    Pair,
    Function,
    Foreign,
}
#[derive(Default)]
pub(crate) struct Roots {
    next: u64,
    pub values: BTreeMap<u64, Value>,
}
pub struct RootValue {
    pub(crate) value: Value,
    pub(crate) roots: Rc<RefCell<Roots>>,
    key: u64,
}
impl RootValue {
    pub(crate) fn new(value: Value, roots: &Rc<RefCell<Roots>>) -> Self {
        let mut registry = roots.borrow_mut();
        let key = registry.next;
        registry.next += 1;
        registry.values.insert(key, value);
        Self {
            value,
            roots: roots.clone(),
            key,
        }
    }
    pub fn integer(&self) -> Option<i64> {
        if let Value::Integer(n) = self.value {
            Some(n)
        } else {
            None
        }
    }
    pub fn boolean(&self) -> Option<bool> {
        if let Value::Boolean(b) = self.value {
            Some(b)
        } else {
            None
        }
    }
}
impl Clone for RootValue {
    fn clone(&self) -> Self {
        Self::new(self.value, &self.roots)
    }
}
impl core::fmt::Debug for RootValue {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RootValue").finish_non_exhaustive()
    }
}
impl Drop for RootValue {
    fn drop(&mut self) {
        self.roots.borrow_mut().values.remove(&self.key);
    }
}
