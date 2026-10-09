#![no_std]
#![no_main]
extern crate alloc;
extern crate programs;
use core::sync::atomic::{AtomicUsize, Ordering};
static COUNT: AtomicUsize = AtomicUsize::new(0);
#[programs::entry]
fn main() -> Result<(), env::Reason> {
    let mut boot = shell_client::Boot::take().map_err(|_| 1usize)?;
    let mut port = boot
        .port("ticks", pipe_client::Direction::Write)
        .map_err(|_| 1usize)?;
    let _worker = execution::unit::task::try_spawn::<_, ()>(|| {
        loop {
            COUNT.fetch_add(1, Ordering::Relaxed);
            env::room::starve();
        }
    })
    .map_err(|_| 1usize)?;
    loop {
        shell_client::write_all(
            &mut port,
            alloc::format!("{}\n", COUNT.load(Ordering::Relaxed)).as_bytes(),
        )
        .map_err(|_| 1usize)?;
        env::room::park(20).map_err(|_| 1usize)?;
    }
}
