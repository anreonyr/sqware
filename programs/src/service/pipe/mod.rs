extern crate alloc;
pub mod core;
pub mod program;
mod serve;
pub fn run() -> Result<(), &'static str> {
    serve::run()
}
