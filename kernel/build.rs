use std::fs;
use std::path::PathBuf;
use std::process::Command;

const UPSTREAM: &str = r#"# This linker script is needed to ensure our version + testcase symbols are not optimized away
# The EMBEDDED_TEST_VERSION symbol is needed by probe-rs to determine whether a binary contains embedded tests or not
# Afterwards it reads the testcases from the .embedded_test section

# Redirect/rename a function here, so that we can make sure the user has added the linker script to the RUSTFLAGS
EXTERN (__embedded_test_start);
PROVIDE(embedded_test_linker_file_not_added_to_rustflags = __embedded_test_start);

PROVIDE(_embedded_test_setup = __embedded_test_default_setup);

# Define a section for the embedded tests and make sure it is not optimized away
SECTIONS
{
  .embedded_test 1 (INFO) :
  {
    KEEP(*(.embedded_test.*));
  }
}

# NOTE: build.rs will add a `INSERT AFTER .comment;` here, if we're compiling for std"#;

const UPSTREAM_HASH: &str = "40c1d2c464932a91c717e13539f4f58716da11f584b84e9719cf1f268ca01b59";

fn upstream_source() -> Option<PathBuf> {
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))?;
    for index in fs::read_dir(cargo_home.join("registry/src"))
        .ok()?
        .flatten()
    {
        let at = index
            .path()
            .join("embedded-test-linker-script-0.1.0/embedded-test.x");
        if at.is_file() {
            return Some(at);
        }
    }
    None
}

fn sha256(of: &[u8]) -> Option<String> {
    let mut child = Command::new("sha256sum")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    use std::io::Write;
    child.stdin.as_mut()?.write_all(of).ok()?;
    let out = child.wait_with_output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    text.split_whitespace().next().map(str::to_string)
}

fn main() {
    let ld = format!("{}/link.x", env!("CARGO_MANIFEST_DIR"));
    println!("cargo::rustc-link-arg=-T{ld}");
    println!("cargo::rerun-if-changed=link.x"); // link.x 变更自动重链

    let hash = sha256(UPSTREAM.as_bytes()).expect("sha256sum 不可用");
    if hash != UPSTREAM_HASH {
        panic!(
            "embedded-test.x 那份抄件与 `UPSTREAM_HASH` 对不上：\n  \
             这次算得 {hash}\n  记为 {} \n  \
             去看一眼上游那份（源文件：{}），确认改了哪几行，再把 UPSTREAM / UPSTREAM_HASH 一起更新。",
            UPSTREAM_HASH,
            upstream_source()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| {
                    "<CARGO_HOME>/registry/src/*/embedded-test-linker-script-0.1.0/embedded-test.x"
                        .into()
                }),
        );
    }
    if let Some(src) = upstream_source() {
        println!("cargo::rerun-if-changed={}", src.display());
        match fs::read(&src).and_then(|real| Ok((sha256(&real), real == UPSTREAM.as_bytes()))) {
            Ok((Some(_), true)) => {}
            Ok((Some(real_hash), false)) => panic!(
                "上游那份 `embedded-test.x` 与本地抄件不同（上游 {real_hash} / 本地 {hash}）。\
                 两份脚本静默漂移正是这枚符号要防的事——请核对 {} 后更新 UPSTREAM 与 UPSTREAM_HASH。",
                src.display()
            ),
            _ => {}
        }
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("embedded-test.x");
    fs::write(&out, UPSTREAM).expect("写 embedded-test.x 失败");
    println!("cargo::rustc-link-arg-tests=-T{}", out.display());
}
