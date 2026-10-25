#![no_std]
#![no_main]

#[mold::entry]
fn main() -> execution::boot::Report<'static> {
    execution::boot::Report::note(env::EXIT_OK, link_dep::NOTE)
}
