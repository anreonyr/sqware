use super::super::{
    adapt::ports::LocalPort,
    core::{Command, JobId, Plan},
};
use alloc::vec::Vec;
use lisp::{ForeignId, RootValue};
pub enum Object {
    Port(LocalPort),
    Command(Command),
    Plan { plan: Plan, roots: Vec<RootValue> },
    Job(JobId),
}
impl Object {
    fn weight(&self) -> usize {
        match self {
            Self::Port(port) => core::mem::size_of::<Self>() + port.bytes().map_or(0, <[u8]>::len),
            Self::Command(command) => command.weight(),
            Self::Plan { plan, roots } => {
                core::mem::size_of::<Self>()
                    + plan.weight()
                    + roots.len() * core::mem::size_of::<RootValue>()
            }
            Self::Job(_) => core::mem::size_of::<Self>(),
        }
    }
}
struct Slot {
    generation: u32,
    value: Option<Object>,
}
#[derive(Default)]
pub struct Objects {
    slots: Vec<Slot>,
    free: Vec<usize>,
}
impl Objects {
    pub fn insert(&mut self, value: Object) -> Result<ForeignId, &'static str> {
        if value.weight() > (4usize * 1024 * 1024).saturating_sub(self.used()) {
            return Err("shell object memory limit exceeded");
        }
        let index = if let Some(index) = self.free.pop() {
            index
        } else {
            if self.slots.len() >= 4096 {
                return Err("object limit exceeded");
            }
            self.slots
                .try_reserve(1)
                .map_err(|_| "object allocation failed")?;
            self.slots.push(Slot {
                generation: 0,
                value: None,
            });
            self.slots.len() - 1
        };
        let slot = &mut self.slots[index];
        slot.value = Some(value);
        Ok(ForeignId {
            slot: index as u32,
            generation: slot.generation,
        })
    }
    pub fn get(&self, id: ForeignId) -> Option<&Object> {
        self.slots
            .get(id.slot as usize)
            .filter(|slot| slot.generation == id.generation)?
            .value
            .as_ref()
    }
    pub fn get_mut(&mut self, id: ForeignId) -> Option<&mut Object> {
        self.slots
            .get_mut(id.slot as usize)
            .filter(|slot| slot.generation == id.generation)?
            .value
            .as_mut()
    }
    pub fn remove(&mut self, id: ForeignId) -> Option<Object> {
        let slot = self
            .slots
            .get_mut(id.slot as usize)
            .filter(|slot| slot.generation == id.generation)?;
        let value = slot.value.take()?;
        if let Some(generation) = slot.generation.checked_add(1) {
            slot.generation = generation;
            self.free.push(id.slot as usize);
        }
        Some(value)
    }
    pub fn key(id: ForeignId) -> u64 {
        ((id.generation as u64) << 32) | id.slot as u64
    }
    pub fn id(key: u64) -> ForeignId {
        ForeignId {
            slot: key as u32,
            generation: (key >> 32) as u32,
        }
    }
    pub fn port(&self, key: u64) -> Option<&LocalPort> {
        match self.get(Self::id(key))? {
            Object::Port(port) => Some(port),
            _ => None,
        }
    }
    pub fn port_mut(&mut self, key: u64) -> Option<&mut LocalPort> {
        match self.get_mut(Self::id(key))? {
            Object::Port(port) => Some(port),
            _ => None,
        }
    }
    pub fn used(&self) -> usize {
        self.slots
            .iter()
            .filter_map(|slot| slot.value.as_ref())
            .map(Object::weight)
            .sum()
    }
    pub fn has_job(&self, id: JobId) -> bool {
        self.slots
            .iter()
            .any(|slot| matches!(slot.value, Some(Object::Job(job)) if job == id))
    }
}
