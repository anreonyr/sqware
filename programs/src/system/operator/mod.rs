mod connection;
pub(crate) mod management;
mod runtime;
mod session;
pub(crate) mod tree;
mod watch;
pub(crate) use runtime::serve as run;

pub(crate) struct Placement {
    pub road: system_api::operator::path::PathBuf,
    pub tile: tree::Tile,
    pub replace: bool,
}

pub(crate) fn connect_task(
    resources: &::schedule::Resources<'_>,
    caller: env::TaskId,
) -> Result<(), &'static str> {
    resources
        .write::<management::Tree>()
        .map_err(|_| "operator admission not installed")?
        .request_connection(caller)
}
pub(crate) fn connect(
    control: ::schedule::Res<crate::system::control::unit::Control>,
    mut tree: ::schedule::ResMut<management::Tree>,
) -> Result<::schedule::Progress, &'static str> {
    for caller in control.tasks() {
        tree.request_connection(caller)?;
    }
    tree.connect()?;
    Ok(::schedule::Progress::Done)
}
