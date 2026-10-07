use crate::{
    PieToken, TaskId, Wait,
    rpc::{Contract, Fail as RpcFail},
    system::{identity::{self, Grant, Reply, Wire}, operator::client::Face as OperatorFace},
    test_state,
};
use system_api::identity::{
    Binding, CoalitionId, Cursor, Fail, Page, PageTarget, PrincipalId, Subject,
};
use wire::Message;

use identity::client::{CallError, Face};

fn authority() -> TaskId { TaskId::new(4) }
fn entry() -> PieToken { PieToken::mint(71) }
fn valid_face() -> Face {
    test_state::authorize(entry(), TaskId::new(2), authority(), Grant::Resolve.mark());
    Face::direct(authority(), Grant::Resolve, entry()).unwrap()
}
fn reply(value: Reply) {
    test_state::respond(Ok(value));
}

#[test]
fn direct_validates_authority_and_grant_mark() {
    test_state::reset();
    let token = entry();
    test_state::authorize(token, TaskId::new(3), authority(), Grant::Resolve.mark());
    assert!(Face::direct(authority(), Grant::Resolve, token).is_ok());

    assert_eq!(Face::direct(TaskId::new(8), Grant::Resolve, token).unwrap_err(), CallError::WrongAuthority);
    test_state::authorize(token, TaskId::new(3), authority(), Grant::Waive.mark());
    assert_eq!(Face::direct(authority(), Grant::Resolve, token).unwrap_err(), CallError::WrongGrant);
    assert_eq!(Face::direct(TaskId::new(0), Grant::Resolve, token).unwrap_err(), CallError::WrongAuthority);
    assert_eq!(Face::direct(authority(), Grant::Resolve, PieToken::mint(99)).unwrap_err(), CallError::Transport);
}

#[test]
fn discover_validates_the_discovered_authority_and_grant() {
    test_state::reset();
    let token = entry();
    let operator = OperatorFace { entry: Some(token) };
    test_state::authorize(token, TaskId::new(2), authority(), Grant::Resolve.mark());
    assert!(Face::discover(&operator, authority(), Grant::Resolve, Wait::POLL).is_ok());

    test_state::authorize(token, TaskId::new(2), TaskId::new(9), Grant::Resolve.mark());
    assert_eq!(Face::discover(&operator, authority(), Grant::Resolve, Wait::POLL).unwrap_err(), CallError::WrongAuthority);
    test_state::authorize(token, TaskId::new(2), authority(), Grant::Waive.mark());
    assert_eq!(Face::discover(&operator, authority(), Grant::Resolve, Wait::POLL).unwrap_err(), CallError::WrongGrant);
    assert_eq!(Face::discover(&OperatorFace { entry: None }, authority(), Grant::Resolve, Wait::POLL).unwrap_err(), CallError::Discovery);
}

#[test]
fn call_rejects_wrong_grant_before_transport_and_revalidates_entry() {
    test_state::reset();
    let face = valid_face();
    assert_eq!(face.call(Wire::Waive, Wait::POLL), Err(CallError::WrongGrant));
    assert!(test_state::sent_calls().is_empty());

    test_state::authorize(entry(), TaskId::new(2), TaskId::new(10), Grant::Resolve.mark());
    assert_eq!(face.call(Wire::Resolve(TaskId::new(12)), Wait::POLL), Err(CallError::WrongAuthority));
    assert!(test_state::sent_calls().is_empty());
}

#[test]
fn call_maps_wrong_source_decode_transport_and_service_failures() {
    test_state::reset();
    let face = valid_face();
    test_state::respond(Err(RpcFail::WrongSource));
    assert_eq!(face.call(Wire::Resolve(TaskId::new(3)), Wait::AtMost(21)), Err(CallError::WrongAuthority));

    test_state::respond(Err(RpcFail::Decode));
    assert_eq!(face.call(Wire::Resolve(TaskId::new(3)), Wait::POLL), Err(CallError::Malformed));

    test_state::respond(Err(RpcFail::Other));
    assert_eq!(face.call(Wire::Resolve(TaskId::new(3)), Wait::POLL), Err(CallError::Transport));

    test_state::respond(Ok(Reply::Fail(Fail::Denied)));
    assert_eq!(face.call(Wire::Resolve(TaskId::new(3)), Wait::POLL), Err(CallError::Service(Fail::Denied)));
}

#[test]
fn successful_call_binds_encoded_request_and_reply_route() {
    test_state::reset();
    let face = valid_face();
    reply(Reply::Principal(Some(PrincipalId::root(authority()))));
    assert_eq!(face.call(Wire::Resolve(TaskId::new(3)), Wait::AtMost(40)), Ok(Reply::Principal(Some(PrincipalId::root(authority())))));
    let sent = test_state::sent_calls();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].entry, entry());
    assert_eq!(sent[0].built_back, sent[0].extracted_back);
    assert_eq!(sent[0].wait, Wait::AtMost(40));
}

#[test]
fn foreign_principal_binding_coalition_and_page_values_are_rejected() {
    test_state::reset();
    let face = valid_face();
    let foreign = TaskId::new(5);

    reply(Reply::Principal(Some(PrincipalId::root(foreign))));
    assert_eq!(face.call(Wire::Resolve(TaskId::new(3)), Wait::POLL), Err(CallError::WrongAuthority));

    let current = Subject::new(PrincipalId::root(foreign), &[]).unwrap();
    let foreign_origin = Subject::new(PrincipalId::root(foreign), &[]).unwrap();
    reply(Reply::Binding(Some(Binding { origin: foreign_origin, current })));
    assert_eq!(face.call(Wire::Resolve(TaskId::new(3)), Wait::POLL), Err(CallError::WrongAuthority));

    reply(Reply::Coalition(CoalitionId::root(foreign)));
    assert_eq!(face.call(Wire::Resolve(TaskId::new(3)), Wait::POLL), Err(CallError::WrongAuthority));

    let members = Page::new(&[PrincipalId::new(foreign, 2)], None).unwrap();
    reply(Reply::Members(members));
    assert_eq!(face.call(Wire::Resolve(TaskId::new(3)), Wait::POLL), Err(CallError::WrongAuthority));

    let memberships = Page::new(&[CoalitionId::new(foreign, 2)], None).unwrap();
    reply(Reply::Memberships(memberships));
    assert_eq!(face.call(Wire::Resolve(TaskId::new(3)), Wait::POLL), Err(CallError::WrongAuthority));
}

#[test]
fn valid_page_cursor_is_bound_to_the_same_authority() {
    test_state::reset();
    let face = valid_face();
    let target = CoalitionId::new(authority(), 6);
    let page = Page::new(&[PrincipalId::new(authority(), 12)], Some(Cursor {
        target: PageTarget::Members(target), revision: 9, after: 12,
    })).unwrap();
    reply(Reply::Members(page));
    assert_eq!(face.call(Wire::Resolve(TaskId::new(3)), Wait::POLL), Ok(Reply::Members(page)));
}

#[test]
fn mismatched_cursor_payload_is_rejected_before_a_request_is_sent() {
    test_state::reset();
    test_state::authorize(entry(), TaskId::new(2), authority(), Grant::Members.mark());
    let face = Face::direct(authority(), Grant::Members, entry()).unwrap();
    let target = CoalitionId::new(authority(), 6);
    let wrong_target = CoalitionId::new(authority(), 7);
    let cursor = Cursor { target: PageTarget::Members(wrong_target), revision: 1, after: 3 };
    let result = face.call(Wire::Members(target, Some(cursor)), Wait::POLL);
    assert_eq!(result, Err(CallError::Transport));
    assert!(test_state::sent_calls().is_empty());
}

#[test]
fn back_contract_preserves_the_route_when_wire_decode_returns_none() {
    test_state::reset();
    let back = PieToken::mint(1234);
    let mut bytes = system_api::identity::Request::EMPTY;
    bytes[..PieToken::WIDTH].copy_from_slice(&back.to_bytes());
    bytes[PieToken::WIDTH] = u8::MAX;
    let (wire, decoded_back) = system_api::identity::Request::fetch(&bytes[..PieToken::WIDTH + 1]).unwrap();
    assert_eq!(wire, None);
    assert_eq!(decoded_back, back);
    assert_eq!(<identity::rpc::Contract as Contract>::back(&(wire, decoded_back)), back);
}
