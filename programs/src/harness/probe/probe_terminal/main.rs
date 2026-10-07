#![no_std]
#![no_main]
extern crate programs;

use env::{MailFail, Mark, Permission, PieFail, PieToken, TaskId, TeamId, Wait, pie, unit};
use ipc::session::{Session, establish};
use protocol::service::terminal::{Connection, Terminal, frame};
use system_client::operator::{self, Face};
use ::resource::raw::{Hole, inspect};
use execution::{room, unit as task};

extern "C" fn unused(_: usize) -> ! {
    room::reap(0, None)
}
fn held() -> TaskId {
    task::spawn(TeamId::new(0), unused as *const () as usize, &[], 0).unwrap()
}
fn denied(input: PieToken, output: PieToken, control: PieToken) {
    let mut bytes = [0; frame::Input::LEN];
    for token in [input, control] {
        assert!(
            matches!(Hole::from_raw(token).pull(&mut bytes, Wait::POLL), Err(e) if e.source == MailFail::Denied)
        );
    }
    assert!(
        matches!(Hole::from_raw(output).push(b"blocked", Wait::POLL), Err(e) if e.source == MailFail::Denied)
    );
}
#[programs::entry]
fn main() -> programs::Report<'static> {
    let session = Session::open(unit::sire(), operator::client::BERTH, Wait::AtMost(1000))
        .ok()
        .expect("probe-terminal: session");
    let terminal = Terminal::find(&Face::of(session)).unwrap();
    let mut connection = Connection::open(terminal).unwrap();
    assert!(connection.lend(TaskId::new(usize::MAX)).is_err());
    connection.io().unwrap().drain().unwrap();
    let host = connection.host();
    let find = |mark| establish::find(host, mark).unwrap();
    let (input, output, control) = (
        find(frame::INPUT),
        find(frame::OUTPUT),
        find(frame::CONTROL),
    );
    let authority = (0..)
        .map(|index| pie::collect(index))
        .take_while(|(token, _, _)| *token != PieToken::NONE)
        .find_map(|(token, _, mark)| {
            (mark == frame::AUTHORITY
                && inspect(token).is_ok_and(|(_, owner, _)| owner == unit::self_id()))
            .then_some(token)
        })
        .unwrap();
    let io = connection.io().unwrap();
    let child = held();
    let foreground = connection.lend(child).unwrap();
    denied(input, output, control);
    assert!(io.read().is_err());
    assert!(io.write(b"blocked").is_err());
    assert!(
        matches!(env::tole::await_(authority, Wait::POLL), Err(e) if e.source == env::ToleFail::HandedOver)
    );
    assert!(
        matches!(pie::accord(authority, child, Permission::FETCH | Permission::VEST | Permission::ONLY, Mark::NONE), Err(e) if e.source == PieFail::HandedOver)
    );
    foreground.restore().unwrap();
    denied(input, output, control);
    connection.io().unwrap().drain().unwrap();
    unit::slay(child).unwrap();
    while !unit::join(child, Wait::Forever).unwrap() {}
    let child = held();
    let foreground = connection.lend(child).unwrap();
    unit::slay(child).unwrap();
    while !unit::join(child, Wait::Forever).unwrap() {}
    foreground.restore().unwrap();
    connection.io().unwrap().drain().unwrap();
    connection.close().unwrap();
    programs::Report::note(
        0,
        "probe-terminal: control handover and data revocation held",
    )
}
