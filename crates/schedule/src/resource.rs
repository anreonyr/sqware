use alloc::{boxed::Box, vec::Vec};
use core::{
    any::{Any, TypeId},
    cell::{Ref, RefCell, RefMut},
};

pub type Res<'a, T> = Ref<'a, T>;
pub type ResMut<'a, T> = RefMut<'a, T>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessError {
    Missing,
    Borrowed,
    Duplicate,
    Room,
}

enum Value<'r> {
    Owned(Box<dyn Any>),
    Write(&'r mut dyn Any),
    Read(&'r dyn Any),
}
impl Value<'_> {
    fn any(&self) -> &dyn Any {
        match self {
            Self::Owned(v) => &**v,
            Self::Write(v) => &**v,
            Self::Read(v) => *v,
        }
    }
    fn any_mut(&mut self) -> Option<&mut dyn Any> {
        match self {
            Self::Owned(v) => Some(&mut **v),
            Self::Write(v) => Some(&mut **v),
            Self::Read(_) => None,
        }
    }
}
struct Entry<'r> {
    id: TypeId,
    value: RefCell<Value<'r>>,
}
/// Typed owned values and scoped borrows; each system releases its guards on return.
pub struct Resources<'r> {
    entries: Vec<Entry<'r>>,
}
impl<'r> Resources<'r> {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
    pub fn insert<T: 'static>(&mut self, value: T) -> Result<&mut Self, AccessError> {
        if self.entries.iter().any(|e| e.id == TypeId::of::<T>()) {
            return Err(AccessError::Duplicate);
        }
        self.entries.try_reserve(1).map_err(|_| AccessError::Room)?;
        self.entries.push(Entry {
            id: TypeId::of::<T>(),
            value: RefCell::new(Value::Owned(Box::new(value))),
        });
        Ok(self)
    }
    pub fn borrow<T: 'static>(&mut self, value: &'r mut T) -> Result<&mut Self, AccessError> {
        self.bind(TypeId::of::<T>(), Value::Write(value))
    }
    pub fn observe<T: 'static>(&mut self, value: &'r T) -> Result<&mut Self, AccessError> {
        self.bind(TypeId::of::<T>(), Value::Read(value))
    }
    fn bind(&mut self, id: TypeId, value: Value<'r>) -> Result<&mut Self, AccessError> {
        if self.entries.iter().any(|e| e.id == id) {
            return Err(AccessError::Duplicate);
        }
        self.entries.try_reserve(1).map_err(|_| AccessError::Room)?;
        self.entries.push(Entry {
            id,
            value: RefCell::new(value),
        });
        Ok(self)
    }
    pub(crate) fn index(&self, id: TypeId, cached: usize) -> Result<usize, AccessError> {
        if self.entries.get(cached).is_some_and(|entry| entry.id == id) {
            return Ok(cached);
        }
        self.entries
            .iter()
            .position(|entry| entry.id == id)
            .ok_or(AccessError::Missing)
    }
    pub fn read<T: 'static>(&self) -> Result<Res<'_, T>, AccessError> {
        self.read_at(self.index(TypeId::of::<T>(), usize::MAX)?)
    }
    pub fn write<T: 'static>(&self) -> Result<ResMut<'_, T>, AccessError> {
        self.write_at(self.index(TypeId::of::<T>(), usize::MAX)?)
    }
    pub(crate) fn read_at<T: 'static>(&self, index: usize) -> Result<Res<'_, T>, AccessError> {
        let e = self.entries.get(index).ok_or(AccessError::Missing)?;
        let r = e.value.try_borrow().map_err(|_| AccessError::Borrowed)?;
        Ref::filter_map(r, |v| v.any().downcast_ref()).map_err(|_| AccessError::Missing)
    }
    pub(crate) fn write_at<T: 'static>(&self, index: usize) -> Result<ResMut<'_, T>, AccessError> {
        let e = self.entries.get(index).ok_or(AccessError::Missing)?;
        let r = e
            .value
            .try_borrow_mut()
            .map_err(|_| AccessError::Borrowed)?;
        RefMut::filter_map(r, |v| v.any_mut().and_then(|v| v.downcast_mut()))
            .map_err(|_| AccessError::Missing)
    }
}

#[derive(Clone, Copy)]
pub struct Access {
    pub(crate) id: TypeId,
    pub(crate) write: bool,
}
pub trait Param: 'static {
    type Item<'a>;
    fn get<'a>(resources: &'a Resources<'_>, index: usize) -> Result<Self::Item<'a>, AccessError>;
    fn access() -> Access;
}
impl<T: 'static> Param for Ref<'static, T> {
    type Item<'a> = Ref<'a, T>;
    fn get<'a>(r: &'a Resources<'_>, index: usize) -> Result<Self::Item<'a>, AccessError> {
        r.read_at(index)
    }
    fn access() -> Access {
        Access {
            id: TypeId::of::<T>(),
            write: false,
        }
    }
}
impl<T: 'static> Param for RefMut<'static, T> {
    type Item<'a> = RefMut<'a, T>;
    fn get<'a>(r: &'a Resources<'_>, index: usize) -> Result<Self::Item<'a>, AccessError> {
        r.write_at(index)
    }
    fn access() -> Access {
        Access {
            id: TypeId::of::<T>(),
            write: true,
        }
    }
}
