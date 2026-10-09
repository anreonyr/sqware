//! Task output and canonical input meet at bounded UART output chunks.
use super::{E_TERMINAL, server::Server};
use crate::core::mode::{ECHO_MAX, Input, Mode};
use env::Wait;
use programs::driver::uart::{client::Console, core::frame::{Bytes, MAX}};
use ::schedule::{Progress, ResMut};
use terminal_api::frame as stream;
use ::resource::raw::Hole;

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
    if frame.n != 0 { server.active = true; return Ok(Progress::Done); }
    let Some((token, foreground)) = server.attachment.as_ref().map(|a| (a.endpoints.output, a.foreground)) else { return Ok(Progress::Done); };
    let mut bytes = [0; stream::MAX];
    if let Ok((n, from)) = Hole::from_raw(token).pull(&mut bytes, Wait::POLL) {
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
    if server.pending.is_empty() && frame.n == 0 {
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
        programs::debug::put(&alloc::format!("terminal: rx gap skipped={skipped}"));
    }
    if server.attachment.is_none() { mode.reset(); return Ok(Progress::Done); }
    for &b in batch.bytes() {
        if b == 3 || b == 26 {
            mode.reset();
            server.reset();
            server.event = Some(if b == 3 { stream::INTERRUPT } else { stream::SUSPEND });
            if server.echo { frame.append(if b == 3 { b"^C\r\n" } else { b"^Z\r\n" }); }
            continue;
        }
        let effect = mode.feed(b);
        if server.echo { frame.append(effect.echo()); }
        match effect.input {
            Input::Line(line) => {
                let mut chunks = line.chunks(stream::MAX).peekable();
                while let Some(chunk) = chunks.next() {
                    if chunks.peek().is_none() && b != 4 && chunk.len() < stream::MAX {
                        let mut bytes = [0; stream::MAX]; bytes[..chunk.len()].copy_from_slice(chunk); bytes[chunk.len()] = b'\n';
                        server.pending.push_back(stream::Input::data(&bytes[..chunk.len() + 1]).ok_or(E_TERMINAL)?);
                    } else { server.pending.push_back(stream::Input::data(chunk).ok_or(E_TERMINAL)?); }
                }
                if b != 4 && line.len().is_multiple_of(stream::MAX) { server.pending.push_back(stream::Input::data(b"\n").ok_or(E_TERMINAL)?); }
            }
            Input::Eof => { server.pending.push_back(stream::Input::eof()); }
            Input::More => {}
        }
    }
    Ok(Progress::Done)
}
pub(super) fn flush(mut console: ResMut<Console>, mut frame: ResMut<Frame>, mut counters: ResMut<Counters>, mut server: ResMut<Server>) -> Result<Progress, env::Reason> {
    let mut sent = 0;
    for chunk in frame.echo[..frame.n].chunks(MAX) {
        if !console.tx.send_when_ready(&Bytes::of(chunk).ok_or(E_TERMINAL)?).map_err(|_| E_TERMINAL)? { break; }
        sent += chunk.len();
    }
    let remaining = frame.n;
    frame.echo.copy_within(sent..remaining, 0);
    frame.n -= sent;
    if frame.n != 0 { server.active = true; }
    let lost = console.tx.lost();
    if lost != counters.lost {
        counters.lost = lost;
        programs::debug::put(&alloc::format!("terminal: tx lost={lost}"));
    }
    Ok(Progress::Done)
}
