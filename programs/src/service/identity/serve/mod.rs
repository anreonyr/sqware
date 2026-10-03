//! Seventeen distinct capabilities, one carrier/Pile, one atomic state transition.
use alloc::vec::Vec;
use env::TaskId;
use protocol::communication::hand::Sender;
use protocol::service::identity::{self as api, Grant, Reply, Wire};
use runtime::core::res::port::{self, Access, Policy};
use runtime::env::mail::{self, HolePie};

use crate::service::identity::core::IdentityBook;
use crate::system::common::face::{carrier::carrier, mount};
use crate::system::common::life::service::Start;
use crate::unit::identity::E_IDENTITY;

mod answer;
pub use answer::answer;

pub fn serve() -> Result<(), Start> {
    let installer = runtime::env::unit::sire();
    let mut book = IdentityBook::new(runtime::env::unit::self_id(), installer)
        .map_err(|_| Start::Book(E_IDENTITY))?;
    let mut faces = Vec::new();
    faces
        .try_reserve_exact(Grant::ALL.len())
        .map_err(|_| Start::Room(E_IDENTITY))?;
    for grant in Grant::ALL {
        let (token, _) =
            mount::entry(grant.mark(), grant.name()).map_err(|_| Start::Tree(E_IDENTITY))?;
        // Control publishes the complete mount table and wires Operator. Identity
        // must not synchronously call Operator while its own query loop is stopped.
        port::ship(
            &HolePie::from_token(token),
            installer,
            Access::FETCH | Access::STORE,
            Policy::VEST,
        )
        .map_err(|_| Start::Tree(E_IDENTITY))?;
        faces.push((token, grant));
    }
    let _ = protocol::communication::session::establish::endpoint(
        installer,
        env::Mark::of(crate::unit::READY),
        env::Wait::POLL,
    );
    carrier(E_IDENTITY, &faces, |grant, from, frame| {
        turn(&mut book, from, grant, frame)
    })
}

fn turn(book: &mut IdentityBook, from: TaskId, grant: Grant, frame: &[u8]) {
    let Some((wire, back)) = Wire::take(frame) else {
        return;
    };
    if !matches!(mail::reserve(back), Ok((_, owner, mark))
        if owner == from && mark == api::BACK)
    {
        return;
    }
    let reply = answer(book, from, grant, wire);
    let _ = Sender::<Reply>::from_token(back).send(reply);
    let _ = mail::release(back);
}
