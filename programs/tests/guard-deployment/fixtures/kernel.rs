use std::cell::Cell;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error {
    pub source: crate::UnitFail,
}

pub mod chrono {
    use super::Cell;

    thread_local! {
        static NOW: Cell<u64> = const { Cell::new(0) };
    }

    pub fn clock() -> u64 {
        NOW.with(Cell::get)
    }

    pub fn set(value: u64) {
        NOW.with(|now| now.set(value));
    }
}

pub mod room {
    pub fn doom(_: crate::TaskId) -> Result<(), ()> {
        crate::record("doom");
        Ok(())
    }
}

pub mod raw {
    pub fn inspect(token: crate::PieToken) -> Result<crate::PieInfo, ()> {
        crate::EFFECTS.with(|effects| {
            effects
                .borrow()
                .inspect
                .unwrap_or(Err(()))
                .map(|mut info| {
                    info.token = token;
                    info
                })
        })
    }
}

pub mod debug {
    pub fn put(_: &str) {}
}

pub mod ledger {
    pub mod manifest {
        pub const MAX_PROGRAMS: usize = 64;
    }
}
