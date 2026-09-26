//! programs 的构建脚本：只做一件事——把镜像链接脚本交给 rustc。
//!
//! **照实记（这里曾经生成过入口胶水）**：`#[entry]` 那个过程宏落地之前，每个 bin 的
//! 入口 shim 是这里按 `[[bin]]` 生成到 `OUT_DIR`、再由各 `main.rs` `include!` 进来的。
//! 换成过程宏之后那一套整个撤了：入口那一手现在长在**你写 `main` 的那个文件里**
//! （宏展开，见 `crates/mold`），既没有 `OUT_DIR` 路径，也没有 29 份生成物。

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
    println!("cargo::rerun-if-changed=src/user/echo.rs");
    println!("cargo::rerun-if-changed=src/driver/router/main.rs");
    println!("cargo::rerun-if-changed=src/driver/uart/main.rs");
    println!("cargo::rerun-if-changed=src/driver/rtc/main.rs");
    println!("cargo::rerun-if-changed=src/system/main.rs");
    println!("cargo::rerun-if-changed=src/root/main.rs");
}
