//! The single program link script; dependents read its path from build metadata.

fn main() {
    let ld = format!("{}/link.ld", env!("CARGO_MANIFEST_DIR"));
    println!("cargo::metadata=linker_script={ld}");
    println!("cargo::rerun-if-changed=link.ld");
}
