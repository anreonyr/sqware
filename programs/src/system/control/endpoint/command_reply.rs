use alloc::vec::Vec;
use ipc::rpc::reply::Sender;
use system_api::control::frame::{OK, Said};

pub(super) fn send(
    reply: Option<Sender<Said>>,
    waiters: Vec<Sender<Said>>,
    said: Said,
) -> bool {
    let mut delivered = false;
    if let Some(reply) = reply {
        delivered |= reply.send(said).is_ok();
    }
    for waiter in waiters {
        delivered |= waiter.send(said).is_ok();
    }
    delivered && said.status == OK
}
