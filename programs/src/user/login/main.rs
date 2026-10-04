#![no_std]
#![no_main]
//! A name prompt that hands its terminal attachment to a cat task.
extern crate alloc;
extern crate programs;

mod cat;

use env::{Wait, unit};
use protocol::communication::session::Session;
use protocol::service::terminal::{Connection, Read, Terminal};
use protocol::system::operator::{self, Face};

#[programs::entry]
fn main() -> Result<(), env::Reason> {
    run().map_err(|step| {
        protocol::debug::put(step);
        1
    })
}
fn run() -> Result<(), &'static str> {
    let session = Session::open(unit::sire(), operator::client::BERTH, Wait::AtMost(1000))
        .map_err(|_| "login: operator")?;
    let terminal = Terminal::find(&Face::of(session)).map_err(|_| "login: terminal lookup")?;
    let mut connection = Connection::open(terminal).map_err(|_| "login: attach")?;
    let mut io = connection.io().map_err(|_| "login: io")?;
    loop {
        io.write(b"login: ").map_err(|_| "login: write")?;
        let name = match io.read().map_err(|_| "login: read")? {
            Read::Data(data) => {
                let bytes = data.bytes();
                let end = bytes
                    .iter()
                    .position(|b| *b == b'\n')
                    .unwrap_or(bytes.len());
                if end == 0 {
                    continue;
                }
                bytes[..end].to_vec()
            }
            Read::Interrupt => {
                io.write(b"\n").map_err(|_| "login: write")?;
                continue;
            }
            Read::Eof => {
                io.drain().map_err(|_| "login: drain")?;
                connection.close().map_err(|_| "login: detach")?;
                return Ok(());
            }
        };
        io.write(b"Hello, ").map_err(|_| "login: write")?;
        io.write(&name).map_err(|_| "login: write")?;
        io.write(b".\n").map_err(|_| "login: write")?;
        io.drain().map_err(|_| "login: drain")?;
        let (task, foreground) = cat::start(&mut connection).map_err(|_| "login: start cat")?;
        protocol::debug::put(&alloc::format!("login: cat task={}", task.get()));
        while !unit::join(task, Wait::Forever).map_err(|_| "login: join cat")? {}
        foreground.restore().map_err(|_| "login: foreground")?;
        io = connection.io().map_err(|_| "login: io")?;
        io.write(b"\n").map_err(|_| "login: write")?;
        protocol::debug::put("login: foreground restored");
    }
}
