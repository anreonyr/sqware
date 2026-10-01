//! programs 的构建脚本：只做一件事——把镜像链接脚本交给 rustc。

fn main() {
    // 镜像程序固定链接在 IMAGE_BASE (0x10000)，见 link.ld。
    // 本 crate 的每个 [[bin]] 都由内核 `build.rs` 经嵌套 cargo 构建并打进 initrd。
    let ld = format!("{}/link.ld", env!("CARGO_MANIFEST_DIR"));
    println!("cargo::rustc-link-arg=-T{ld}");
    println!("cargo::rerun-if-changed=link.ld");

    // **每个 bin 的源都要盯**：本 crate 的 `[[bin]]` 那些文件**不在** Cargo 的默认重编扫描里
    // （默认只盯 `src/lib.rs` 那一族），源码改了它可能不重编 ⇒ 下游（`crates/image`、门）
    // 拿到的是**旧产物**。实测栽过：`cargo image` 打出旧 initrd，量出来的东西其实不是刚改的。
    // 一条一条列（不走 `src` 目录的 `rerun-if-changed`：那是未定义行为），让 cargo 自己算指纹。
    println!("cargo::rerun-if-changed=src/lib.rs");
    println!("cargo::rerun-if-changed=src/user/canonical/main.rs");
    println!("cargo::rerun-if-changed=src/driver/router/main.rs");
    println!("cargo::rerun-if-changed=src/driver/uart/main.rs");
    println!("cargo::rerun-if-changed=src/driver/rtc/main.rs");
    println!("cargo::rerun-if-changed=src/system/main.rs");
    println!("cargo::rerun-if-changed=src/root/main.rs");
}
