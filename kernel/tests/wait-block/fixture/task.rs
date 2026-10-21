use super::*;

pub struct Task {
    pub ident: ident::Ident,
}

impl Task {
    pub fn new() -> Self { Self { ident: ident::Ident { id: env::TaskId::new(1) } } }
    pub fn running_task(&self) -> Option<Arc<Self>> {
        PRESENT.get().then(|| Arc::new(Self::new()))
    }
    pub fn swap(&self) -> (Arc<Self>, Option<usize>) {
        assert!(COMMIT.try_lock().is_err(), "swap outside commit");
        record("swap");
        (Arc::new(Self::new()), Some(42))
    }
    pub fn block(&self, _: WakeKey, _: Ticket, _: Option<WaitCall>) {
        assert!(!DOOM.get(), "Waiting overwrote Slay");
        assert!(COMMIT.try_lock().is_err(), "publish outside commit");
        record("block");
    }
}
