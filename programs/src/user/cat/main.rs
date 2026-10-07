#![no_std]
#![no_main]
extern crate alloc;
extern crate programs;
use env::{TaskId, Wait, unit};
use terminal_client::{Io, Read};
use system_client::identity::{Grant, Reply, Wire, client::Face};
#[programs::entry]
fn main() -> Result<(), env::Reason> {
    run().map_err(|_| 1)
}
fn run() -> Result<(), ()> {
    let owner = TaskId::new(*execution::boot::args::args().first().ok_or(())?);
    let io = Io::injected(owner)?;
    let entry = ::resource::raw::pies()
        .find(|pie| pie.mark == Grant::Resolve.mark())
        .ok_or(())?;
    let resolver = Face::direct(entry.owner, Grant::Resolve, entry.token).map_err(|_| ())?;
    let Reply::Binding(Some(binding)) = resolver
        .call(Wire::Resolve(unit::self_id()), Wait::AtMost(1000))
        .map_err(|_| ())?
    else {
        return Err(());
    };
    io.write(
        alloc::format!(
            "cat: task={} principal={}:{}\n",
            unit::self_id().get(),
            binding.current.principal.authority.get(),
            binding.current.principal.slot
        )
        .as_bytes(),
    )?;
    loop {
        match io.read()? {
            Read::Data(data) => io.write(data.bytes())?,
            Read::Eof | Read::Interrupt => {
                io.drain()?;
                return Ok(());
            }
        }
    }
}
