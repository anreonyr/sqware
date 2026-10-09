#![no_std]
#![no_main]
extern crate programs;
#[programs::entry]
fn main() -> Result<(), env::Reason> {
    let _boot = shell_client::Boot::take().map_err(|_| 1usize)?;
    loop {
        env::room::starve();
    }
}
