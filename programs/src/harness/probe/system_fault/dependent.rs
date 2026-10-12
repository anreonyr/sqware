#![no_std]
#![no_main]
extern crate alloc;
extern crate programs;
use env::{TaskId, Wait};
use ipc::session::establish;
use programs::harness::probe::hierarchy::{ANSWER, COMMAND};
use resource::raw::Hole;
use system_api::identity::{Grant, Reply, Wire};
const WAIT: Wait = Wait::AtMost(3000);
#[programs::entry]
fn main() -> programs::Report<'static> {
    let control = env::unit::sire();
    let me = env::unit::self_id();
    let command = establish::claim(control, COMMAND, WAIT).unwrap();
    let answer = establish::claim(control, ANSWER, WAIT).unwrap();
    let authority = system_client::identity::authority().unwrap();
    let resolve = system_client::identity::Face::direct(
        authority,
        Grant::Resolve,
        establish::find(authority, Grant::Resolve.mark()).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        resolve.call(Wire::Resolve(me), WAIT),
        Ok(Reply::Binding(Some(_)))
    ));
    let forged = env::pie::unseal(env::UnsealArgs::hole(Grant::Resolve.mark())).unwrap();
    assert!(
        system_client::identity::Face::direct(authority, Grant::Resolve, forged).is_err(),
        "a locally forged role Mark cannot replace the Identity authority"
    );
    env::pie::release(forged, env::ReleaseMode::Revoke).unwrap();
    let _ready = establish::Held(
        establish::lend(control, programs::unit::READY_MARK).unwrap(),
    );
    loop {
        let mut bytes = [0; 17];
        let (n, sender) = Hole::from_raw(command)
            .pull(&mut bytes, Wait::Forever)
            .unwrap();
        assert_eq!((n, sender), (bytes.len(), control));
        let task = TaskId::new(u64::from_le_bytes(bytes[1..9].try_into().unwrap()) as usize);
        match bytes[0] {
            1 => {
                let Reply::Binding(Some(own)) = resolve.call(Wire::Resolve(me), WAIT).unwrap()
                else {
                    panic!("own identity missing")
                };
                let Reply::Binding(Some(child)) = resolve.call(Wire::Resolve(task), WAIT).unwrap()
                else {
                    panic!("child identity missing")
                };
                assert_eq!(own.current.principal, child.current.principal);
                assert_eq!(own.current.principal.authority, authority);
            }
            3 => assert!(matches!(
                resolve.call(Wire::Resolve(task), WAIT),
                Ok(Reply::Binding(None))
            )),
            _ => panic!("unknown hierarchy command"),
        }
        Hole::from_raw(answer).push(&bytes[..1], WAIT).unwrap();
    }
}
