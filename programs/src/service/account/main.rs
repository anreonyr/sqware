#![no_std]
#![no_main]
extern crate programs;
#[programs::entry]
fn main() -> Result<(), env::Reason> {
    programs::service::account::run().map_err(|why| {
        programs::debug::put(why);
        25
    })
}
