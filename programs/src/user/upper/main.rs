#![no_std]
#![no_main]
extern crate programs;
#[programs::entry]
fn main() -> Result<(), env::Reason> {
    let mut boot = shell_client::Boot::take().map_err(|_| 1usize)?;
    let source = boot
        .port("source", pipe_client::Direction::Read)
        .map_err(|_| 1usize)?;
    let result = boot
        .port("result", pipe_client::Direction::Write)
        .map_err(|_| 1usize)?;
    shell_client::copy(source, result, true).map_err(|_| 1usize)
}
