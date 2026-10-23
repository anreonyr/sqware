use core::cell::Cell;

thread_local! {
    static NOW: Cell<u64> = const { Cell::new(1_000) };
}

pub fn clock() -> u64 {
    NOW.with(|now| now.get())
}

pub fn set(value: u64) {
    NOW.with(|now| now.set(value));
}
