#![no_std]
#![no_main]
extern crate programs;
#[programs::entry]
fn main() -> Result<(), env::Reason> {
    programs::user::shell::run().map_err(|message| {
        programs::debug::put(message);
        86
    })
}
