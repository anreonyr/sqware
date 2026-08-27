fn main() {
    // 用户程序固定链接在 IMAGE_BASE (0x10000)，见 link.ld。
    let ld = format!("{}/link.ld", env!("CARGO_MANIFEST_DIR"));
    println!("cargo::rustc-link-arg=-T{ld}");
    println!("cargo::rerun-if-changed=link.ld");
}
