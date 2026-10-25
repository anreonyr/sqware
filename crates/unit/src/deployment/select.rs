use crate::{DeploymentSelectionError, UnitFile};

use super::Deployment as DeploymentValue;

/// Validate a catalog selection against explicit deployment and Unit declaration slices.
pub fn select<'a>(
    deployments: &'a [DeploymentValue],
    units: &[&'static UnitFile],
    name: &str,
    entry: env::ledger::manifest::Entry<'_>,
) -> Result<&'a DeploymentValue, DeploymentSelectionError> {
    let deployment = DeploymentValue::find(deployments, name).ok_or(DeploymentSelectionError::Unknown)?;
    let program = units
        .iter()
        .copied()
        .find(|program| program.name() == deployment.entry)
        .ok_or(DeploymentSelectionError::EntryMismatch)?;
    if entry.name != program.loader.image || entry.kind != program.loader.space {
        return Err(DeploymentSelectionError::EntryMismatch);
    }
    Ok(deployment)
}
