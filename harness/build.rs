//! harness 的构建脚本：只做一件事——与 `programs` 共用同一张链接脚本。
//!
//! **照实记（这里曾经生成过入口胶水）**：与 `programs/build.rs` 同款，那一套随
//! `#[entry]` 过程宏一起撤了（见 `crates/entry-macro` 的头注）。

fn main() {
    // 与 `programs` **同一张链接脚本**（镜像程序都链在 IMAGE_BASE = 0x10000，见那份
    // `link.ld` 头注）。测具与产品是同一批"被内核装载的 ELF"，链接口径没有第二套。
    let ld = format!("{}/../programs/link.ld", env!("CARGO_MANIFEST_DIR"));
    println!("cargo::rustc-link-arg=-T{ld}");
    println!("cargo::rerun-if-changed={ld}");
}
