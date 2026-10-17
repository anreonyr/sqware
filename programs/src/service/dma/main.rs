#![no_std]
#![no_main]

//! 端点服务那一台的引导镜像：身子在 `..` 的 `endpoint::run`。

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    match programs::service::dma::endpoint::run() {
        Ok(()) => programs::Report::note(env::EXIT_OK, "system-dma: endpoint done"),
        Err(why) => programs::Report::note(env::EXIT_FAULT, why),
    }
}
