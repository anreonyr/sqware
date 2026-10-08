pub(crate) mod management;
mod service;
pub(crate) mod tree;
pub(crate) use service::run::serve as run;

pub(crate) struct Placement {
    pub road: system_api::operator::path::PathBuf,
    pub tile: tree::Tile,
    pub replace: bool,
}
