//! Identity task 独占身份账和请求面，按授权提交身份状态变更。
use alloc::{vec::Vec, sync::Arc};
use core::sync::atomic::Ordering;
use crate::system::life::{Status, Phase};
use env::{HoleDir, Wait};
use runtime::core::res::pile::Pile;
use env::TaskId;
use protocol::communication::hand::Sender;
use protocol::system::identity::{self as api, Grant, Reply, Wire};
use runtime::core::res::port::{self, Access, Policy};
use runtime::env::mail::{self, HolePie};

use crate::system::identity::core::IdentityBook;
use crate::system::common::face::mount;

#[derive(Debug)]
pub enum Fail { Book, Room, Tree, Desk, Dead }

mod answer;
pub use answer::answer;

pub fn serve(status: Arc<Status>) -> Result<(), Fail> {
    let installer = status.control;
    let mut book = IdentityBook::new(runtime::env::unit::self_id(), installer)
        .map_err(|_| Fail::Book)?;
    let mut faces = Vec::new();
    faces.try_reserve_exact(Grant::ALL.len()).map_err(|_| Fail::Room)?;
    for grant in Grant::ALL {
        let (token, _) = mount::entry(grant.mark(), grant.name())
            .map_err(|_| Fail::Tree)?;
        // Control publishes the complete mount table and wires Operator. Identity
        // must not synchronously call Operator while its own query loop is stopped.
        port::ship(
            &HolePie::from_token(token), installer,
            Access::FETCH | Access::STORE, Policy::VEST,
        ).map_err(|_| Fail::Tree)?;
        faces.push((token, grant));
    }
    let pile = Pile::unseal(false).map_err(|_| Fail::Desk)?;
    for (entry, _) in &faces {
        pile.attach(&HolePie::from_token(*entry), HoleDir::Pull).map_err(|_| Fail::Desk)?;
    }
    let mut bytes = alloc::vec![0; runtime::PAGE_SIZE];
    loop {
        if status.phase.load(Ordering::Acquire) == Phase::Stopping as u8 { return Ok(()); }
        let peer = TaskId::new(status.operator.load(Ordering::Acquire));
        if runtime::env::unit::join(installer, Wait::POLL).unwrap_or(true)
            || runtime::env::unit::join(peer, Wait::POLL).unwrap_or(true)
        { return Err(Fail::Dead); }
        let hit = pile.await_(Wait::AtMost(100)).map_err(|_| Fail::Dead)?;
        let Some((entry, _)) = hit else { continue; };
        let Some((_, grant)) = faces.iter().find(|(token, _)| *token == entry) else { continue; };
        while let Ok((n, from)) = HolePie::from_token(entry).pull(&mut bytes, Wait::POLL) {
            turn(&mut book, from, *grant, &bytes[..n]);
        }
    }
}

fn turn(book: &mut IdentityBook, from: TaskId, grant: Grant, frame: &[u8]) {
    let Some((wire, back)) = Wire::take(frame) else { return; };
    if !matches!(mail::reserve(back), Ok((_, owner, mark))
        if owner == from && mark == api::BACK)
    { return; }
    let reply = answer(book, from, grant, wire);
    let _ = Sender::<Reply>::from_token(back).send(reply);
    let _ = mail::release(back);
}

pub mod install;
pub mod source;
pub mod query;
pub mod names;
