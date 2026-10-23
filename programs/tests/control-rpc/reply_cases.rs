use crate::{command_reply, rpc::reply::{self, Sender}};
use system_api::control::frame::{self, Said};

#[test]
fn failed_or_missing_routes_cannot_confirm_release_delivery() {
    let said = frame::said_status(frame::OK);
    assert!(!command_reply::send(None, vec![], said));
    assert!(!command_reply::send(Some(Sender::new(false)), vec![Sender::new(false)], said));
}

#[test]
fn a_successful_waiter_reports_delivery_and_all_routes_are_attempted() {
    reply::reset();
    assert!(command_reply::send(Some(Sender::new(false)),
        vec![Sender::new(true), Sender::new(false)], frame::said_status(frame::OK)));
    assert_eq!(reply::sent(), 3);
}

#[test]
fn delivered_failures_cannot_confirm_successful_release() {
    let said: Said = frame::said_status(frame::DENIED);
    assert!(!command_reply::send(Some(Sender::new(true)), vec![], said));
}
