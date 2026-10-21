use super::*;

pub struct Sites;

impl Sites {
    pub fn lock(&self) -> MutexGuard<'static, BTreeMap<WakeKey, Site>> {
        static TABLE: Mutex<BTreeMap<WakeKey, Site>> = Mutex::new(BTreeMap::new());
        TABLE.lock().unwrap()
    }
}
