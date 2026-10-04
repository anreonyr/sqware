//! Cat is a separate task in the login team. Endpoints are installed before release.
use env::{TaskId, TeamId};
use protocol::service::terminal::{Connection, Foreground, Io, Read};
use runtime::core::task;

pub(super) fn start(connection: &mut Connection) -> Result<(TaskId, Foreground<'_>), ()> {
    let child = task::spawn(
        TeamId::new(0),
        entry as *const () as usize,
        &[connection.host().get()],
        0,
    )
    .map_err(|_| ())?;
    let foreground = match connection.lend(child) {
        Ok(foreground) => foreground,
        Err(()) => {
            let _ = env::unit::slay(child);
            return Err(());
        }
    };
    if env::unit::embark(child).is_err() {
        drop(foreground);
        let _ = env::unit::slay(child);
        return Err(());
    }
    Ok((child, foreground))
}
extern "C" fn entry(args: usize) -> ! {
    // SAFETY: Spawn copied one owner ID into this task's startup argument area.
    let owner = TaskId::new(unsafe { core::ptr::read_volatile(args as *const usize) });
    let tls = task::tls::allocate().expect("cat: tls");
    // SAFETY: this fresh task owns the TLS page for its entire execution.
    unsafe {
        core::arch::asm!("mv tp, {}", in(reg) tls, options(nomem, nostack, preserves_flags));
    }
    let result = run(owner);
    task::tls::deallocate();
    task::exit(if result.is_ok() { 0 } else { 1 }, None)
}
fn run(owner: TaskId) -> Result<(), ()> {
    let io = Io::injected(owner).map_err(|_| {
        protocol::debug::put("cat: endpoint lookup");
        ()
    })?;
    loop {
        match io.read()? {
            Read::Data(data) => io.write(data.bytes())?,
            Read::Eof => {
                io.drain()?;
                protocol::debug::put("cat: eof");
                return Ok(());
            }
            Read::Interrupt => {
                io.drain()?;
                protocol::debug::put("cat: interrupted");
                return Ok(());
            }
        }
    }
}
