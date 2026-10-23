mod control_config;
mod retirement;
mod unit_file;

pub use control_config::ControlConfig;
pub use retirement::Retirement;
pub use unit_file::UnitFile;

pub fn join_task(_: crate::TaskId, _: crate::Wait) -> Result<bool, ()> {
    Ok(false)
}
