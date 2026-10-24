#![no_std]
#![no_main]

//! VirtIO block Driver image: the body lives in `..`'s `serve`.

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    match programs::driver::virtio::blk::serve::serve() {
        Ok(()) => programs::Report::note(env::EXIT_OK, "virtio-blk: done"),
        Err(why) => programs::Report::note(env::EXIT_FAULT, why),
    }
}
