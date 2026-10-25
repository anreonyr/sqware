use super::GuardSelectionError;

/// Validate a deployment's trusted guard selection against its own start list and
/// the program catalog. The names stay in deployment declarations; this function
/// only enforces uniqueness and membership.
pub fn select(
    guarded: &'static [&'static str],
    start: &[&'static str],
    listed: impl Fn(&str) -> bool,
) -> Result<&'static [&'static str], GuardSelectionError> {
    for (index, name) in guarded.iter().enumerate() {
        if guarded[..index].contains(name) {
            return Err(GuardSelectionError::Duplicate);
        }
        if !start.contains(name) {
            return Err(GuardSelectionError::Unstarted);
        }
        if !listed(name) {
            return Err(GuardSelectionError::Unknown);
        }
    }
    Ok(guarded)
}
