use alloc::{boxed::Box, vec::Vec};
use core::{any::{Any, TypeId}, cell::{Ref, RefMut, RefCell}};

pub type Res<'a, T> = Ref<'a, T>;
pub type ResMut<'a, T> = RefMut<'a, T>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessError { Missing, Borrowed, Duplicate, Room }

enum Value<'r> { Owned(Box<dyn Any>), Write(&'r mut dyn Any), Read(&'r dyn Any) }
impl Value<'_> {
    fn any(&self) -> &dyn Any { match self { Self::Owned(v) => &**v, Self::Write(v) => &**v, Self::Read(v) => *v } }
    fn any_mut(&mut self) -> Option<&mut dyn Any> { match self { Self::Owned(v) => Some(&mut **v), Self::Write(v) => Some(&mut **v), Self::Read(_) => None } }
}
struct Entry<'r> { id: TypeId, value: RefCell<Value<'r>> }
/// Typed owned values and scoped borrows; each system releases its guards on return.
pub struct Resources<'r> { entries: Vec<Entry<'r>> }
impl<'r> Resources<'r> {
    pub fn new() -> Self { Self { entries: Vec::new() } }
    pub fn insert<T: 'static>(&mut self, value: T) -> Result<(), AccessError> {
        if self.entries.iter().any(|e| e.id == TypeId::of::<T>()) { return Err(AccessError::Duplicate); }
        self.entries.try_reserve(1).map_err(|_| AccessError::Room)?;
        self.entries.push(Entry { id: TypeId::of::<T>(), value: RefCell::new(Value::Owned(Box::new(value))) });
        Ok(())
    }
    pub fn borrow<T: 'static>(&mut self, value: &'r mut T) -> Result<(), AccessError> {
        self.bind(TypeId::of::<T>(), Value::Write(value))
    }
    pub fn observe<T: 'static>(&mut self, value: &'r T) -> Result<(), AccessError> {
        self.bind(TypeId::of::<T>(), Value::Read(value))
    }
    fn bind(&mut self, id: TypeId, value: Value<'r>) -> Result<(), AccessError> {
        if self.entries.iter().any(|e| e.id == id) { return Err(AccessError::Duplicate); }
        self.entries.try_reserve(1).map_err(|_| AccessError::Room)?;
        self.entries.push(Entry { id, value: RefCell::new(value) }); Ok(())
    }
    pub fn read<T: 'static>(&self) -> Result<Res<'_, T>, AccessError> {
        let e = self.entries.iter().find(|e| e.id == TypeId::of::<T>()).ok_or(AccessError::Missing)?;
        let r = e.value.try_borrow().map_err(|_| AccessError::Borrowed)?;
        Ref::filter_map(r, |v| v.any().downcast_ref()).map_err(|_| AccessError::Missing)
    }
    pub fn write<T: 'static>(&self) -> Result<ResMut<'_, T>, AccessError> {
        let e = self.entries.iter().find(|e| e.id == TypeId::of::<T>()).ok_or(AccessError::Missing)?;
        let r = e.value.try_borrow_mut().map_err(|_| AccessError::Borrowed)?;
        RefMut::filter_map(r, |v| v.any_mut().and_then(|v| v.downcast_mut())).map_err(|_| AccessError::Missing)
    }
}

#[derive(Clone, Copy)]
pub struct Access { pub(crate) id: TypeId, pub(crate) write: bool }
pub trait Param: 'static {
    type Item<'a>;
    fn get<'a>(resources: &'a Resources<'_>) -> Result<Self::Item<'a>, AccessError>;
    fn access() -> Access;
}
impl<T: 'static> Param for Ref<'static, T> {
    type Item<'a> = Ref<'a, T>;
    fn get<'a>(r: &'a Resources<'_>) -> Result<Self::Item<'a>, AccessError> { r.read() }
    fn access() -> Access { Access { id: TypeId::of::<T>(), write: false } }
}
impl<T: 'static> Param for RefMut<'static, T> {
    type Item<'a> = RefMut<'a, T>;
    fn get<'a>(r: &'a Resources<'_>) -> Result<Self::Item<'a>, AccessError> { r.write() }
    fn access() -> Access { Access { id: TypeId::of::<T>(), write: true } }
}
