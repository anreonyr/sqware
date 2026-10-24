use core::cell::Cell;

use env::PieToken;

thread_local! {
    static OK: Cell<bool> = const { Cell::new(true) };
    static GIVES: Cell<usize> = const { Cell::new(0) };
    static RELEASES: Cell<usize> = const { Cell::new(0) };
    static RELEASE_OK: Cell<bool> = const { Cell::new(true) };
    static PUSH_OK: Cell<bool> = const { Cell::new(true) };
}

pub fn reset() {
    OK.with(|ok| ok.set(true));
    GIVES.with(|gives| gives.set(0));
    RELEASES.with(|value| value.set(0));
    RELEASE_OK.with(|value| value.set(true));
    PUSH_OK.with(|value| value.set(true));
}

pub fn fail() {
    OK.with(|ok| ok.set(false));
}

pub fn gives() -> usize {
    GIVES.with(|gives| gives.get())
}

pub fn push_ok(okay: bool) { PUSH_OK.with(|value| value.set(okay)); }
pub fn release_ok(okay: bool) { RELEASE_OK.with(|value| value.set(okay)); }
pub fn releases() -> usize { RELEASES.with(Cell::get) }
pub fn release() -> Result<(), ()> {
    if !RELEASE_OK.with(Cell::get) { return Err(()); }
    RELEASES.with(|value| value.set(value.get() + 1));
    Ok(())
}

pub fn give() -> Result<PieToken, ()> {
    GIVES.with(|gives| gives.set(gives.get() + 1));
    if OK.with(|ok| ok.get()) {
        Ok(PieToken::mint(0x99))
    } else {
        Err(())
    }
}

pub struct Hole;

impl Hole {
    pub fn from_raw(_: PieToken) -> Self {
        Self
    }

    pub fn push(&self, _: &[u8], _: env::Wait) -> Result<(), ()> {
        if PUSH_OK.with(Cell::get) { Ok(()) } else { Err(()) }
    }
}
