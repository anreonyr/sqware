use ::schedule::{Progress, Res, ResMut};

/// The System-owned Operator layout, bootstrapped before any Unit is prepared.
pub(crate) type Layout = Option<crate::system::app::namespace::System>;

pub(crate) fn bootstrap(
    tree: Res<crate::system::operator::management::Tree>,
    mut layout: ResMut<Layout>,
) -> Result<Progress, &'static str> {
    if layout.is_some() {
        return Ok(Progress::Done);
    }
    let info = tree.root_entry()?;
    *layout = Some(
        crate::system::app::namespace::System::bootstrap(
            info,
            env::Wait::AtMost(crate::support::timing::BOOT_MS),
        )
        .map_err(|_| "operator root namespace")?,
    );
    Ok(Progress::Done)
}
