#![no_std]
#![no_main]
extern crate programs;

use env::{MailFail, Mark, Permission, PieFail, PieToken, TaskId, TeamId, Wait, pie, unit};
use ipc::session::{Session, establish};
use terminal_client::{Connection, Terminal};
use terminal_api::frame;
use system_client::operator;
use system_client::operator::Face;
use ::resource::raw::Hole;
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
    let session = Session::open(unit::sire(), operator::BERTH, Wait::AtMost(1000))
        .ok()
        .expect("probe-terminal: session");
    let terminal = Terminal::find(&Face::of(session)).unwrap();
    let mut connection = Connection::open(terminal).unwrap();
    assert!(connection.lend(TaskId::new(usize::MAX)).is_err());
    connection.io().unwrap().drain().unwrap();
    let [input, output, control] = connection.io().unwrap().raw_channels();
    let authority = establish::claim(unit::self_id(), frame::AUTHORITY, Wait::AtMost(1000))
        .expect("probe-terminal: exclusive authority");
    let io = connection.io().unwrap();
    let child = held();
    let foreground = connection.lend(child).unwrap();
    denied(input, output, control);
    assert!(io.read().is_err());
    assert!(io.write(b"blocked").is_err());
    assert!(
        matches!(env::mail::await_(authority, Wait::POLL), Err(e) if e.source == env::MailFail::HandedOver)
    );
    assert!(
        matches!(pie::accord(authority, child, Permission::FETCH | Permission::VEST | Permission::ONLY, Mark::NONE), Err(e) if e.source == PieFail::HandedOver)
    );
    foreground.restore().unwrap();
    denied(input, output, control);
    connection.io().unwrap().drain().unwrap();
    unit::slay_task(child).unwrap();
    while !unit::join_task(child, Wait::Forever).unwrap() {}
    let child = held();
    let foreground = connection.lend(child).unwrap();
    unit::slay_task(child).unwrap();
    while !unit::join_task(child, Wait::Forever).unwrap() {}
    foreground.restore().unwrap();
    connection.io().unwrap().drain().unwrap();
    connection.close().unwrap();
    programs::Report::note(
        0,
        "probe-terminal: control handover and data revocation held",
    )
}
