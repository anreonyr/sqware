use super::{E_TERMINAL, frame::{Counters, Frame}, schedule, server::Server};
use crate::core::mode::Mode;
use programs::driver::uart::client::Console;
use ::schedule::{Cursor, Progress, Resources};

pub fn run(console: &mut Console) -> Result<(), env::Reason> {
    let server = Server::open(console)?;
    let mut resources = Resources::new();
    resources.insert(Counters::of(console)).map_err(|_| E_TERMINAL)?;
    resources.borrow(console).map_err(|_| E_TERMINAL)?;
    resources.insert(Mode::canonical()).map_err(|_| E_TERMINAL)?;
    resources.insert(Frame::new()).map_err(|_| E_TERMINAL)?;
    resources.insert(server).map_err(|_| E_TERMINAL)?;
    let mut frame = schedule::frame().map_err(|_| E_TERMINAL)?;
    frame.prepare(&resources);
    let mut cursor = Cursor::default();
    loop {
        let progress = frame.advance(&mut cursor, &resources).map_err(|_| E_TERMINAL)?;
        if !resources.read::<Server>().map_err(|_| E_TERMINAL)?.running { return Ok(()); }
        if progress == Progress::Done { cursor.reset(); }
    }
}
