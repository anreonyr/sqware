//! 故障注入景那一台的引导镜像：身子与 `system` 同一台（`app::run`），
//! 只是所在景的装配在第二轮装一次“第一次停住答 `Pending`”。

#![no_std]
#![no_main]
extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    match programs::system::app::run() {
        Ok(()) => programs::Report::note(env::EXIT_OK, "system: done"),
        Err(reason) => programs::Report::note(reason, "system: failed"),
    }
}
