#![no_std]
#![no_main]

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    let _ready = ipc::session::establish::Held(
        ipc::session::establish::endpoint(
            env::unit::sire(),
            programs::unit::READY_MARK,
            env::Wait::POLL,
        )
        .expect("system-child: ready"),
    );
    loop {
        execution::room::park(core::time::Duration::from_millis(100)).expect("system-child: wait");
    }
}
