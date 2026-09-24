//! `sqware-image [场景] [档]` —— 造镜像那一步的命令行（默认 `root` / `release`）。
//!
//! 交互式跑之前先来这一下（`.cargo/config.toml` 的别名：`cargo image`）——`cargo run` 本身
//! 只管编内核，不再顺带造镜像（那是"initrd 与 kernel 何干"那一问的落点）。

fn main() {
    let mut args = std::env::args().skip(1);
    let scenario = args.next().unwrap_or_else(|| "root".to_string());
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
