#![no_std]
#![no_main]
extern crate programs;
#[programs::entry]
fn main() -> Result<(), env::Reason> {
    let mut boot = shell_client::Boot::take().map_err(|_| 1usize)?;
    let mut port = boot
        .port("records", pipe_client::Direction::Write)
        .map_err(|_| 1usize)?;
    if boot.args.first().is_some_and(|arg| arg == "--repeat") {
        let count = boot
            .args
            .get(1)
            .and_then(|arg| arg.parse::<usize>().ok())
            .filter(|n| *n <= 1048576)
            .ok_or(2usize)?;
        let text = boot.args.get(2).ok_or(2usize)?;
        for _ in 0..count {
            shell_client::write_all(&mut port, text.as_bytes()).map_err(|_| 1usize)?;
        }
    } else {
        for arg in &boot.args {
            shell_client::write_all(&mut port, arg.as_bytes()).map_err(|_| 1usize)?;
            shell_client::write_all(&mut port, b"\n").map_err(|_| 1usize)?;
        }
    }
    port.close().map_err(|_| 1usize)
}
