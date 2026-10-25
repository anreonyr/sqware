extern crate alloc;
pub mod core;
mod serve;
pub fn run() -> Result<(), &'static str> {
    serve::run()
}
