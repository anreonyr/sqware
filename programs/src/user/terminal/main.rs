#![no_std]
#![no_main]
//! User terminal, initially operating in canonical input mode.

extern crate alloc;
extern crate programs;

mod adapt;
use programs::user::terminal::core;

use adapt::{E_NO_CONSOLE, MS};
use env::{Wait, unit};
use programs::driver::uart::client;
use ipc::{rack::Mode, session::Session};
use system_client::operator::{self, Face};

#[programs::entry]
fn main() -> Result<(), env::Reason> {
    let session = Session::open(unit::sire(), operator::client::BERTH, Wait::AtMost(MS))
        .map_err(|_| E_NO_CONSOLE)?;
    let road = client::road().ok_or(E_NO_CONSOLE)?;
    let mut console = client::find(&Face::of(session), &road, Mode::Oldest, Wait::AtMost(MS))
        .ok_or(E_NO_CONSOLE)?;
    adapt::run::run(&mut console)
}
