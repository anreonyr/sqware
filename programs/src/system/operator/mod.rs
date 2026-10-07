mod claim;
pub mod core;
pub mod serve;
pub mod client;

pub struct Placement {
    pub road: system_api::operator::path::PathBuf,
    pub tile: core::Tile,
    pub replace: bool,
}
