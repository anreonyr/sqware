use super::*;

pub struct Site {
    pub life: Weak<Life>,
    pub pend: bool,
}

impl Site {
    pub fn ensure(sites: &mut BTreeMap<WakeKey, Self>, key: WakeKey, life: &Weak<Life>) -> Result<(), ()> {
        record("ensure");
        sites.entry(key).or_insert(Self { life: life.clone(), pend: false });
        Ok(())
    }
    pub fn push_back(&mut self, _: Arc<Task>) { record("enqueue"); }
}
