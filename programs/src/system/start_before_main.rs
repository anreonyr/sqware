//! 起手受控失败那一景（`spawn` 之后、发布之前）的引导镜像：身子与 `system` 同一台
//! （`app::run`），只是所在景的装配在那个受控点停一次。
//!
//! 它与 `system-start-after-dma` 各是一张**不同镜像**（`crates/image` 按入口认景），
//! 故两个 bin 各占一份文件。

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
