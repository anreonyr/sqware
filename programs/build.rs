//! programs 的构建脚本：只做一件事——把镜像链接脚本交给 rustc。
//!
//! **照实记（这里曾经生成过入口胶水）**：`#[entry]` 那个过程宏落地之前，每个 bin 的
//! 入口 shim 是这里按 `[[bin]]` 生成到 `OUT_DIR`、再由各 `main.rs` `include!` 进来的。
//! 换成过程宏之后那一套整个撤了：入口那一手现在长在**你写 `main` 的那个文件里**
//! （宏展开，见 `crates/entry-macro`），既没有 `OUT_DIR` 路径，也没有 29 份生成物。

fn main() {
    // 镜像程序固定链接在 IMAGE_BASE (0x10000)，见 link.ld。
    // 本 crate 的每个 [[bin]] 都由内核 `build.rs` 经嵌套 cargo 构建并打进 initrd。
    let ld = format!("{}/link.ld", env!("CARGO_MANIFEST_DIR"));
    println!("cargo::rustc-link-arg=-T{ld}");
    println!("cargo::rerun-if-changed=link.ld");
}
