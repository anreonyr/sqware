//! `sqware-image [场景] [档]` —— 造镜像那一步的命令行（默认 `product` / `release`）。
//!
//! 交互式跑之前先来这一下（`.cargo/config.toml` 的别名：`cargo image`）——`cargo run` 本身
//! 只管编内核，不再顺带造镜像（那是"initrd 与 kernel 何干"那一问的落点）。
//!
//! **照实记（默认那一景从 `root` 改成 `product`，量出来的）**：`root` 是**验收景**——它多带
//! **15 台测具**（`guest` / `passer` / `lodger` / `sleeper` / `subject` / `member` 与八台
//! `probe-*`，声明在 `programs/src/decl/harness.rs`，`scenes` 走 `Identity::DEFAULT`）。
//! 默认成 `root` 的那一版里，**一发 `cargo image`（release 档、不带参数）就把这 15 台打进
//! `initrd.img`**：25 台 / 1239036 B，而产品景是 **10 台 / 570961 B**。验收那条路自己写景名
//! （`cargo image root release`、`nu scripts/qtest.nu --scene root`），一个字都不受影响。

fn main() {
    let mut args = std::env::args().skip(1);
    let scenario = args.next().unwrap_or_else(|| "product".to_string());
    let profile = args.next().unwrap_or_else(|| "release".to_string());
    if let Some(extra) = args.next() {
        eprintln!("image: 多出来的参数 `{extra}`（用法：sqware-image [场景] [档]）");
        std::process::exit(2);
    }
    match image::build(&scenario, &profile) {
        Ok(at) => println!("ok  {}", at.display()),
        Err(why) => {
            eprintln!("image: {why}");
            std::process::exit(1);
        }
    }
}
