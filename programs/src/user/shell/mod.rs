pub mod adapt;
pub mod core;
pub mod native;
mod repl;
pub fn run() -> Result<(), &'static str> {
    repl::run()
}
