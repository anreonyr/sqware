#![no_std]
#![no_main]
extern crate programs;
#[programs::entry]
fn main() -> Result<(), env::Reason> {
    programs::service::pipe::run().map_err(|message| {
        programs::debug::put(message);
        85
    })
}
