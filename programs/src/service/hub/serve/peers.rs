use env::{PieToken, Wait};
use hub_api::coordination::{Call, call};
use ipc::session::establish;
use system_api::control::{self, Grant};
use system_api::operator::Path;
use system_client::control::Face as ControlFace;
use system_client::operator::Face;

use crate::driver::shared::peer::Peer;

pub struct Peers {
    pub coord: Peer<Call>,
    pub control: ControlFace,
    _state: ControlFace,
}

impl Peers {
    /// Failed construction retains every imported reference in `held` for retry.
    pub fn find(tree: &Face, wait: Wait, held: &mut [Option<PieToken>; 3]) -> Result<Self, &'static str> {
        if held.iter().any(Option::is_some) {
            return Err("hub: endpoint cleanup pending");
        }
        let state_road = control::DIR.try_join(Grant::State.name()).ok_or("hub: state road")?;
        let state_entry = lookup(tree, &state_road, wait)?;
        held[0] = Some(state_entry);
        let state = ControlFace::of_owned(state_entry, env::unit::sire())
            .map_err(|_| "hub: state owner")?;
        if establish::marked_as(state_entry) != Some(Grant::State.mark()) {
            return Err("hub: state mark");
        }
        let instance_entry = lookup(tree, control::INSTANCE, wait)?;
        held[1] = Some(instance_entry);
        let control = ControlFace::of_owned(instance_entry, env::unit::sire())
            .map_err(|_| "hub: instance owner")?;
        if establish::marked_as(instance_entry) != Some(system_api::control::ASK_MARK) {
            return Err("hub: instance mark");
        }
        let task = state.publisher(system_api::namespace::SERVICE, call::ENTRY, wait)
            .map_err(|_| "hub: driver publisher")?;
        let entry = lookup(tree, Path::new(call::ENTRY_PATH), wait)?;
        held[2] = Some(entry);
        let coord = Peer::<Call>::open_checked(entry, (task, call::ENTRY), Call::BACK)?;
        held.fill(None);
        Ok(Self { coord, control, _state: state })
    }
}

pub(super) fn reclaim(held: &mut [Option<PieToken>; 3]) {
    for slot in held {
        if let Some(token) = *slot {
            if env::pie::release(token, env::ReleaseMode::Revoke).is_ok() {
                *slot = None;
            }
        }
    }
}

fn lookup(tree: &Face, road: &Path, wait: Wait) -> Result<PieToken, &'static str> {
    let service = tree.resolve(system_api::namespace::SERVICE, wait)
        .map_err(|_| "hub: endpoint namespace")?;
    tree.tile_from(service, road, wait).and_then(|tile| tile.token(wait))
        .map_err(|_| "hub: endpoint lookup")
}
