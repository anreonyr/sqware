#![no_std]
#![no_main]
extern crate alloc;
extern crate programs;
mod auth;
use env::{Wait, unit};
use protocol::communication::session::Session;
use protocol::service::terminal::{Connection, Io, Read, Terminal};
use protocol::system::{
    control::{Face as Lifecycle, State, account::Client},
    operator::{self, Face},
};
use zeroize::Zeroizing;
const WAIT: Wait = Wait::AtMost(5000);
#[programs::entry]
fn main() -> Result<(), env::Reason> {
    run().map_err(|step| {
        protocol::debug::put(step);
        1
    })
}
fn password(
    connection: &Connection,
    io: &Io,
) -> Result<Option<Zeroizing<alloc::vec::Vec<u8>>>, ()> {
    connection.set_echo(false)?;
    let result = (|| {
        io.write(b"Password: ")?;
        match io.read()? {
            Read::Data(mut input) => {
                let bytes = input.bytes();
                let password = bytes
                    .strip_suffix(b"\n")
                    .map(|bytes| Zeroizing::new(bytes.to_vec()));
                input.clear();
                Ok(password)
            }
            Read::Eof | Read::Interrupt => Ok(None),
        }
    })();
    let restored = connection.set_echo(true);
    io.write(b"\n")?;
    restored?;
    result
}
fn run_cat(
    connection: &mut Connection,
    client: &Client,
    lifecycle: &Lifecycle,
) -> Result<bool, ()> {
    let built = match client.create(auth::ACCOUNT, WAIT) {
        Ok(built) => built,
        Err(_) => return Ok(false),
    };
    let foreground = match connection.lend(built.task) {
        Ok(foreground) => foreground,
        Err(_) => {
            lifecycle.instance(built.task).ruin(WAIT).map_err(|_| ())?;
            return Ok(false);
        }
    };
    let run: Result<(), ()> = (|| {
        lifecycle
            .instance(built.task)
            .embark(WAIT)
            .map_err(|_| ())?;
        loop {
            if lifecycle.instance(built.task).state(WAIT).map_err(|_| ())? == State::Dead {
                break;
            }
            execution::room::sleep(core::time::Duration::from_millis(10)).map_err(|_| ())?;
        }
        Ok(())
    })();
    let stopped = if run.is_err() {
        lifecycle.instance(built.task).ruin(WAIT).map_err(|_| ())
    } else {
        Ok(())
    };
    let restored = foreground.restore();
    stopped?;
    restored?;
    Ok(run.is_ok())
}
fn run() -> Result<(), &'static str> {
    let session = Session::open(unit::sire(), operator::client::BERTH, WAIT)
        .map_err(|_| "login: operator")?;
    let tree = Face::of(session);
    let client = Client::find(&tree, WAIT).map_err(|_| "login: account endpoint")?;
    let lifecycle = Lifecycle::of(
        tree.tile(protocol::system::control::client::INSTANCE, WAIT)
            .and_then(|tile| tile.token(WAIT))
            .map_err(|_| "login: instance endpoint")?,
    )
    .map_err(|_| "login: lifecycle")?;
    let terminal = Terminal::find(&tree).map_err(|_| "login: terminal lookup")?;
    let mut connection = Connection::open(terminal).map_err(|_| "login: attach")?;
    let mut io = connection.io().map_err(|_| "login: io")?;
    loop {
        io.write(b"login: ").map_err(|_| "login: write")?;
        let account = match io.read().map_err(|_| "login: read")? {
            Read::Data(data) => {
                let bytes = data.bytes();
                let name = bytes.strip_suffix(b"\n").unwrap_or(bytes);
                if name.is_empty() {
                    continue;
                }
                name.to_vec()
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
        let Some(secret) = password(&connection, &io).map_err(|_| "login: password")? else {
            continue;
        };
        let authenticated = auth::verify(&account, &secret);
        drop(secret);
        if !authenticated {
            io.write(b"Login incorrect\n").map_err(|_| "login: write")?;
            continue;
        }
        io.write(b"Hello, anran.\n").map_err(|_| "login: write")?;
        io.drain().map_err(|_| "login: drain")?;
        let completed = run_cat(&mut connection, &client, &lifecycle)
            .map_err(|_| "login: instance cleanup or terminal restore")?;
        io = connection.io().map_err(|_| "login: restore io")?;
        io.write(if completed {
            b"\n"
        } else {
            b"Session unavailable\n"
        })
        .map_err(|_| "login: write")?;
    }
}
