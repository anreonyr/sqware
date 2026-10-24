use env::Wait;
use ipc::session::{Session, establish};
use system_api::control::{self, Grant};
use system_client::control::{Face, Observe};
use system_client::operator;

use super::faces::Faces;
use crate::support::timing::BOOT_MS;

/// Open only the public supervision and recover faces below the service namespace.
pub fn open() -> Result<Faces, &'static str> {
    let until = ipc::time::deadline(Wait::AtMost(BOOT_MS));
    let wait = || ipc::time::remain(until);
    let session = Session::open(
        env::unit::sire(),
        operator::BERTH,
        wait(),
    )
    .map_err(|_| "daemon: operator session")?;
    let tree = operator::Face::from(&session);
    let base = tree
        .resolve(system_api::namespace::SERVICE, wait())
        .map_err(|_| "daemon: service namespace")?;
    let observe = tree
        .tile_from(base, control::SUPERVISION, wait())
        .and_then(|tile| tile.token(wait()))
        .map_err(|_| "daemon: supervision face")?;
    let recover = tree
        .tile_from(base, control::RECOVER, wait())
        .and_then(|tile| tile.token(wait()))
        .map_err(|_| "daemon: recover face")?;
    let host = env::unit::sire();
    if establish::opened_by(observe) != Some(host)
        || establish::opened_by(recover) != Some(host)
    {
        return Err("daemon: face host");
    }
    if establish::marked_as(observe) != Some(Grant::Observe.mark())
        || establish::marked_as(recover) != Some(Grant::Recover.mark())
    {
        return Err("daemon: face mark");
    }
    let observe = Observe::of(observe).map_err(|_| "daemon: observe client")?;
    let recover = Face::of(recover).map_err(|_| "daemon: recover client")?;
    Ok(Faces { observe, recover })
}
