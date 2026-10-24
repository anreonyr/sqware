use env::{Mark, PieToken, TaskId, Wait};
use ipc::session::establish;
use system_api::{control::Grant, operator::Path};
use system_client::{control::Face as ControlFace, operator::Face};
use wire::Contract;
use super::peer::Peer;

pub fn reclaim(held: &mut Option<PieToken>) {
    if let Some(token) = *held {
        if env::pie::release(token, env::ReleaseMode::Revoke).is_ok() { *held = None; }
    }
}

pub fn state(tree: &Face, system: TaskId, held: &mut Option<PieToken>, wait: Wait)
    -> Result<ControlFace, &'static str>
{
    if system == TaskId::new(0) { return Err("lookup: expected System"); }
    let road = system_api::control::DIR.try_join(Grant::State.name()).ok_or("lookup: state path")?;
    let token = acquire(tree, &road, held, wait)?;
    if establish::marked_as(token) != Some(Grant::State.mark()) { return Err("lookup: state mark"); }
    let face = ControlFace::of_owned(token, system).map_err(|_| "lookup: state owner")?;
    *held = None;
    Ok(face)
}

pub fn peer<C: Contract>(tree: &Face, state: &ControlFace, road: &Path, mark: Mark, back: Mark,
    held: &mut Option<PieToken>, wait: Wait) -> Result<Peer<C>, &'static str>
{
    if mark == Mark::NONE { return Err("lookup: expected mark"); }
    let task = state.publisher(system_api::namespace::SERVICE, mark, wait)
        .map_err(|_| "lookup: publisher")?;
    let token = acquire(tree, road, held, wait)?;
    let peer = Peer::<C>::open_checked(token, (task, mark), back)?;
    *held = None;
    Ok(peer)
}

fn acquire(tree: &Face, road: &Path, held: &mut Option<PieToken>, wait: Wait)
    -> Result<PieToken, &'static str>
{
    reclaim(held);
    if held.is_some() { return Err("lookup: cleanup pending"); }
    let service = tree.resolve(system_api::namespace::SERVICE, wait).map_err(|_| "lookup: namespace")?;
    let token = tree.tile_from(service, road, wait).and_then(|tile| tile.token(wait))
        .map_err(|_| "lookup: endpoint")?;
    *held = Some(token);
    Ok(token)
}
