//! Task output and canonical input meet at bounded UART output chunks.
use super::{E_TERMINAL, server::Server};
use crate::core::mode::{ECHO_MAX, Input, Mode};
use env::Wait;
use programs::driver::uart::{client::Console, core::frame::{Bytes, MAX}};
use ::schedule::{Progress, ResMut};
use protocol::service::terminal::frame as stream;
use ::resource::raw::HolePie;

pub(super) struct Frame {
    incoming: Option<Bytes>,
    echo: [u8; ECHO_MAX * MAX + 2],
    n: usize,
}
impl Frame {
    pub fn new() -> Self { Self { incoming: None, echo: [0; ECHO_MAX * MAX + 2], n: 0 } }
    fn append(&mut self, bytes: &[u8]) {
        self.echo[self.n..self.n + bytes.len()].copy_from_slice(bytes);
        self.n += bytes.len();
    }
}
pub(super) struct Counters { seen: u64, lost: u64, cr: bool }
impl Counters {
    pub fn of(console: &Console) -> Self { Self { seen: console.rx.skipped(), lost: console.tx.lost(), cr: false } }
}

pub(super) fn output(mut server: ResMut<Server>, mut frame: ResMut<Frame>, mut counters: ResMut<Counters>) -> Result<Progress, env::Reason> {
    let Some((token, foreground)) = server.attachment.as_ref().map(|a| (a.endpoints.output, a.foreground)) else { return Ok(Progress::Done); };
    let mut bytes = [0; stream::MAX];
    if let Ok((n, from)) = HolePie::from_token(token).pull(&mut bytes, Wait::POLL) {
        server.active = true;
        if from == foreground {
            for &b in &bytes[..n] {
                if b == b'\n' && !counters.cr { frame.append(b"\r"); }
                frame.append(&[b]);
                counters.cr = b == b'\r';
            }
        }
    }
    Ok(Progress::Done)
}
pub(super) fn receive(mut console: ResMut<Console>, mut frame: ResMut<Frame>, mut server: ResMut<Server>) -> Result<Progress, env::Reason> {
    frame.incoming = None;
    if server.pending.is_empty() {
        if let Ok(batch) = console.rx.recv(Wait::POLL) {
            frame.incoming = Some(batch);
            server.active = true;
        }
    } else {
        // Delivery wakes the next RX poll. Leave bytes queued but clear its stale bell.
        let _ = console.rx.hush();
    }
    Ok(Progress::Done)
}
pub(super) fn feed(
    console: ResMut<Console>, mut mode: ResMut<Mode>, mut frame: ResMut<Frame>,
    mut counters: ResMut<Counters>, mut server: ResMut<Server>,
) -> Result<Progress, env::Reason> {
    let Some(batch) = frame.incoming.take() else { return Ok(Progress::Done); };
    let skipped = console.rx.skipped();
    if skipped != counters.seen {
        counters.seen = skipped;
        mode.reset();
        protocol::debug::put(&alloc::format!("terminal: rx gap skipped={skipped}"));
    }
    if server.attachment.is_none() { mode.reset(); return Ok(Progress::Done); }
    for &b in batch.bytes() {
        if b == 3 {
            mode.reset();
            server.reset();
            server.interrupt = true;
            if server.echo { frame.append(b"^C\r\n"); }
            continue;
        }
        let effect = mode.feed(b);
        if server.echo { frame.append(effect.echo()); }
        match effect.input {
            Input::Line(line) => {
                let mut bytes = [0; stream::MAX];
                bytes[..line.len()].copy_from_slice(line);
                let mut n = line.len();
                if b != 4 { bytes[n] = b'\n'; n += 1; }
                server.pending.push_back(stream::Input::data(&bytes[..n]).ok_or(E_TERMINAL)?);
            }
            Input::Eof => { server.pending.push_back(stream::Input::eof()); }
            Input::More => {}
        }
    }
    Ok(Progress::Done)
}
pub(super) fn flush(mut console: ResMut<Console>, mut frame: ResMut<Frame>, mut counters: ResMut<Counters>) -> Result<Progress, env::Reason> {
    for chunk in frame.echo[..frame.n].chunks(MAX) {
        console.tx.send(&Bytes::of(chunk).ok_or(E_TERMINAL)?).map_err(|_| E_TERMINAL)?;
    }
    frame.n = 0;
    let lost = console.tx.lost();
    if lost != counters.lost {
        counters.lost = lost;
        protocol::debug::put(&alloc::format!("terminal: tx lost={lost}"));
    }
    Ok(Progress::Done)
}
