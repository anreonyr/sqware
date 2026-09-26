fn main() {
    // 内核链接脚本：workspace 化后不同 crate 用不同脚本（内核 `0x80200000` /
    // 用户 `0x10000`），不能放根 `.cargo/config.toml`（全局 rustflags 冲突），改由本脚本传绝对路径。
    //
    // **照实记（后缀 `ld` → `x`，用户裁定"迁移到 embedded-test"）**：`cargo-qtest` 对 riscv
    // 的判据是"manifest 目录里有没有 `.x` 文件"——没有就自己在 `0x80000000` 生成一份，
    // 而那与 `SBI.bin` 撞 ROM 区（实测 `Some ROM regions are overlapping`）。改名即压掉它。
    let ld = format!("{}/link.x", env!("CARGO_MANIFEST_DIR"));
    println!("cargo::rustc-link-arg=-T{ld}");
    println!("cargo::rerun-if-changed=link.x"); // link.x 变更自动重链
}
