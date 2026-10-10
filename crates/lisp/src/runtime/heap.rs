use super::{
    env::Environment,
    native::ForeignId,
    value::{Handle, Value},
};
use crate::syntax::lower::{ExprId, Module};
use alloc::{rc::Rc, string::String, vec::Vec};
#[derive(Clone)]
pub(crate) enum Object {
    Symbol(String),
    String(String),
    Bytes(Vec<u8>),
    Pair(Value, Value),
    Cell(Value),
    Environment(Environment),
    Closure {
        module: Rc<Module>,
        body: ExprId,
        params: usize,
        env: Option<Handle>,
    },
    Foreign {
        id: ForeignId,
        kind: String,
    },
}
impl Object {
    fn weight(&self) -> usize {
        core::mem::size_of::<Self>()
            + match self {
                Self::Symbol(s) | Self::String(s) => s.capacity(),
                Self::Bytes(b) => b.capacity(),
                Self::Environment(e) => e.cells.capacity() * core::mem::size_of::<Handle>(),
                Self::Closure { module, .. } => module.weight(),
                Self::Foreign { kind, .. } => kind.capacity(),
                _ => 0,
            }
    }
    fn references(&self, out: &mut Vec<Value>) {
        match self {
            Self::Pair(a, b) => {
                out.push(*a);
                out.push(*b);
            }
            Self::Cell(v) => out.push(*v),
            Self::Environment(e) => {
                out.extend(e.parent.map(Value::Object));
                out.extend(e.cells.iter().copied().map(Value::Object));
            }
            Self::Closure { env, .. } => out.extend(env.map(Value::Object)),
            _ => {}
        }
    }
}
struct Slot {
    generation: u32,
    marked: bool,
    object: Option<Object>,
    weight: usize,
}
pub(crate) struct Heap {
    slots: Vec<Slot>,
    free: Vec<usize>,
    pub used: usize,
    pub limit: usize,
    metadata: usize,
}
impl Heap {
    pub fn new(limit: usize) -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            used: 0,
            limit,
            metadata: 0,
        }
    }
    pub fn object_budget(&self) -> usize {
        self.limit.saturating_sub(self.metadata)
    }
    pub fn remaining(&self) -> usize {
        self.object_budget().saturating_sub(self.used)
    }
    pub fn charge(&mut self, weight: usize) -> Result<(), ()> {
        if weight > self.remaining() {
            return Err(());
        }
        self.used += weight;
        Ok(())
    }
    pub fn alloc(&mut self, object: Object) -> Result<Handle, ()> {
        let weight = object.weight();
        if weight > self.remaining() {
            return Err(());
        }
        let index = if let Some(index) = self.free.pop() {
            index
        } else {
            if self.slots.len() == self.slots.capacity() {
                let capacity = self.slots.capacity().max(16).checked_mul(2).ok_or(())?;
                let metadata = capacity
                    .checked_mul(core::mem::size_of::<Slot>() + core::mem::size_of::<usize>())
                    .ok_or(())?;
                let extra = metadata.saturating_sub(self.metadata);
                if extra.checked_add(weight).ok_or(())? > self.remaining() {
                    return Err(());
                }
                self.free
                    .try_reserve_exact(capacity - self.free.len())
                    .map_err(|_| ())?;
                self.metadata = self.slots.capacity() * core::mem::size_of::<Slot>()
                    + self.free.capacity() * core::mem::size_of::<usize>();
                self.slots
                    .try_reserve_exact(capacity - self.slots.len())
                    .map_err(|_| ())?;
                self.metadata = metadata;
            }
            self.slots.push(Slot {
                generation: 0,
                marked: false,
                object: None,
                weight: 0,
            });
            self.slots.len() - 1
        };
        let slot = &mut self.slots[index];
        slot.object = Some(object);
        slot.weight = weight;
        self.used += weight;
        Ok(Handle {
            slot: index,
            generation: slot.generation,
        })
    }
    pub fn get(&self, handle: Handle) -> Option<&Object> {
        self.slots
            .get(handle.slot)
            .filter(|s| s.generation == handle.generation)?
            .object
            .as_ref()
    }
    pub fn get_mut(&mut self, handle: Handle) -> Option<&mut Object> {
        self.slots
            .get_mut(handle.slot)
            .filter(|s| s.generation == handle.generation)?
            .object
            .as_mut()
    }
    pub fn collect(&mut self, mut roots: Vec<Value>) -> Vec<ForeignId> {
        while let Some(value) = roots.pop() {
            let Value::Object(handle) = value else {
                continue;
            };
            let Some(slot) = self.slots.get_mut(handle.slot) else {
                continue;
            };
            if slot.generation != handle.generation || slot.marked {
                continue;
            }
            slot.marked = true;
            if let Some(object) = &slot.object {
                object.references(&mut roots);
            }
        }
        let mut released = Vec::new();
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if slot.marked {
                slot.marked = false;
                continue;
            }
            if let Some(object) = slot.object.take() {
                if let Object::Foreign { id, .. } = object {
                    released.push(id);
                }
                self.used -= slot.weight;
                slot.weight = 0;
                if let Some(generation) = slot.generation.checked_add(1) {
                    slot.generation = generation;
                    self.free.push(index);
                }
            }
        }
        released
    }
}
