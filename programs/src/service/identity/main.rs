#![no_std]
#![no_main]

extern crate programs;

#[programs::entry]
fn main() -> Result<(), programs::system::common::life::service::Start> {
    programs::service::identity::serve::serve()
}
