#![no_std]
#![no_main]
extern crate programs;

#[programs::entry]
fn main() -> Result<(), env::Reason> {
    programs::service::daemon::run().map_err(|why| {
        programs::debug::put(why);
        programs::unit::daemon::E_DAEMON
    })
}
