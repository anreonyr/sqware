use crate::system::operator::tree::{Change, Location, Operator, Tile};
use ::resource::raw::alive;
use alloc::{string::ToString, vec::Vec};
use env::pie;
use env::{PieToken, TaskId};
use system_api::operator::EntryId;
use system_api::operator::Fail;
use system_api::operator::Where;

use crate::system::operator::Placement;

pub(super) fn plate(
    tree: &mut Operator,
    request: &Placement,
) -> Result<(EntryId, Vec<Change>), Fail> {
    let road = &request.road;
    let Tile {
        pie: leaf,
        permit,
        owner,
    } = request.tile;
    let owner = owner.unwrap_or(TaskId::new(0));
    let replace = request.replace;
    let mut fresh = Vec::new();
    fresh.try_reserve(road.len()).map_err(|_| Fail::Full)?;
    let mut changes = Vec::new();
    changes.try_reserve(road.len()).map_err(|_| Fail::Full)?;
    let result = (|| {
        let last = road.file_name().ok_or(Fail::Unknown)?;
        let mut at = Where::Root;
        for seg in road.parent().ok_or(Fail::Unknown)?.iter() {
            let (id, created, change) = tree.part_at(at, seg.to_string())?;
            if created {
                fresh.push(id);
            }
            // An existing tile must never masquerade as a directory.
            let _ = tree.list(Where::At(id))?;
            if let Some(change) = change {
                changes.push(change);
            }
            at = Where::At(id);
        }
        if leaf == PieToken::NONE {
            let (id, created, change) = tree.part_at(at, last.to_string())?;
            let _ = tree.list(Where::At(id))?;
            if created {
                fresh.push(id);
            }
            if let Some(change) = change {
                changes.push(change);
            }
            return Ok(id);
        }
        if !alive(leaf) {
            return Err(Fail::Dead);
        }
        if !replace && tree.kid(at, last)?.is_some() {
            return Err(Fail::Denied);
        }
        let change = tree.land(
            Location {
                at,
                name: last.to_string(),
            },
            Tile {
                pie: leaf,
                permit,
                owner: (owner.get() != 0).then_some(owner),
            },
        )?;
        let id = change.id;
        changes.push(change);
        Ok(id)
    })();
    match result {
        Ok(id) => Ok((id, changes)),
        Err(fail) => {
            for id in fresh.into_iter().rev() {
                let _ = tree.trim(id);
            }
            if leaf != PieToken::NONE {
                let _ = pie::release(leaf, env::ReleaseMode::Keep);
            }
            Err(fail)
        }
    }
}
