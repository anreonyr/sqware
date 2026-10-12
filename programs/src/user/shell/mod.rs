pub mod adapt;
pub(crate) mod commands;
pub mod core;
pub mod native;
mod repl;
mod startup;
pub fn run() -> Result<(), &'static str> {
    repl::run()
}
