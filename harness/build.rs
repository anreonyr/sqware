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

    // **每个 bin 的源都要盯**：本 crate 的 `[[bin]]` 那些文件**不在** Cargo 的默认重编扫描里
    // （默认只盯 `src/lib.rs` 那一族），源码改了它可能不重编 ⇒ 下游（`crates/image`、门）
    // 拿到的是**旧产物**。实测栽过：`cargo image` 打出旧 initrd，量出来的东西其实不是刚改的。
    // 一条一条列（不走 `src` 目录的 `rerun-if-changed`：那是未定义行为），让 cargo 自己算指纹。
    println!("cargo::rerun-if-changed=src/lib.rs");
    println!("cargo::rerun-if-changed=src/probe_lease.rs");
    println!("cargo::rerun-if-changed=src/probe_owner.rs");
    println!("cargo::rerun-if-changed=src/probe_denied.rs");
    println!("cargo::rerun-if-changed=src/probe_rule.rs");
    println!("cargo::rerun-if-changed=src/probe_rule_other.rs");
    println!("cargo::rerun-if-changed=src/guest.rs");
    println!("cargo::rerun-if-changed=src/passer.rs");
    println!("cargo::rerun-if-changed=src/lodger/main.rs");
    println!("cargo::rerun-if-changed=src/sleeper.rs");
    println!("cargo::rerun-if-changed=src/subject.rs");
    println!("cargo::rerun-if-changed=src/member.rs");
    println!("cargo::rerun-if-changed=src/churn.rs");
    println!("cargo::rerun-if-changed=src/rig.rs");
    println!("cargo::rerun-if-changed=src/busy.rs");
    println!("cargo::rerun-if-changed=src/park.rs");
    println!("cargo::rerun-if-changed=src/hang.rs");
    println!("cargo::rerun-if-changed=src/load.rs");
    println!("cargo::rerun-if-changed=src/beat.rs");
    println!("cargo::rerun-if-changed=src/again.rs");
    println!("cargo::rerun-if-changed=src/waiter.rs");
    println!("cargo::rerun-if-changed=src/group.rs");
}
