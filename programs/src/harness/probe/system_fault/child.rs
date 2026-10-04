#![no_std]
#![no_main]

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    let _ready = protocol::communication::session::establish::Held(
        protocol::communication::session::establish::endpoint(
            env::unit::sire(),
            env::Mark::of(programs::unit::READY),
            env::Wait::POLL,
        )
        .expect("system-child: ready"),
    );
    loop {
        runtime::core::task::sleep(core::time::Duration::from_millis(100))
            .expect("system-child: wait");
    }
}
