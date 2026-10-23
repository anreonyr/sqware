use std::cell::Cell;

thread_local! {
    static HART: Cell<usize> = const { Cell::new(0) };
}

pub struct HartId(usize);

impl HartId {
    pub fn get(self) -> usize {
        self.0
    }
}

pub fn hart_id() -> HartId {
    HartId(HART.get())
}

pub fn select(hart: usize) {
    HART.set(hart);
}
