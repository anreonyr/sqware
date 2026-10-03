#![no_std]
#![no_main]

use programs::harness::probe::fixture::Fixture;
use programs::system::run::{bootstrap, scene};

#[programs::entry]
fn main() -> programs::Report<'static> {
    let boot = bootstrap::take().expect("accept: bootstrap");
    let list = scene::programs(&boot.catalog).expect("accept: scene");
    let mut fixture = Fixture::new(boot).ok().expect("accept: setup");
    for program in list {
        fixture.assemble(program).expect("accept: install unit");
    }
    fixture.supervise().expect("accept: shutdown");
    programs::Report::note(env::EXIT_OK, "accept: done")
}
