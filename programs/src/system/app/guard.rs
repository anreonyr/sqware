use env::{Permission, PieKind, PieToken};

/// Validate the boot guard authority for a trusted deployment selection.
/// An empty selection needs no authority; a non-empty one must carry a live
/// FETCH-only Nole owned by the System task.
pub(crate) fn authority(
    guarded: &'static [&'static str],
    token: Option<PieToken>,
) -> Result<Option<PieToken>, &'static str> {
    if guarded.is_empty() {
        return Ok(None);
    }
    let token = token.ok_or("guard authority missing")?;
    let info = resource::raw::inspect(token).map_err(|_| "guard authority unavailable")?;
    if !info.alive
        || info.kind != PieKind::Nole
        || info.permission != Permission::FETCH
        || info.owner != env::unit::self_id()
    {
        return Err("guard authority");
    }
    Ok(Some(token))
}
