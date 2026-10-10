#![no_std]
#![no_main]
extern crate programs;
#[programs::entry]
fn main() -> Result<(), env::Reason> {
    let boot = shell_client::Boot::take().map_err(|_| 1usize)?;
    Err(boot
        .args
        .first()
        .and_then(|arg| arg.parse().ok())
        .unwrap_or(17))
}
