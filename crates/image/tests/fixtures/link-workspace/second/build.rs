fn main() {
    let ld = std::env::var("DEP_EXECUTION_LINKER_SCRIPT")
        .expect("execution did not publish DEP_EXECUTION_LINKER_SCRIPT");
    println!("cargo::rustc-link-arg=-T{ld}");
    println!("cargo::rerun-if-changed={ld}");
}
