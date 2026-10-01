//! **内核那两个目标的链接脚本**：bin 一枚、test 一枚（各自的 `-T` 从哪来）。
//!
//! # 两枚脚本，两个目标
//!
//! ```text
//!   -T<link.x>              本仓那一枚：给出 `0x80200000` 的布局与
//!                           `_kernel_base` / `_kernel_edge` / `_canary` / `_stack`
//!   -T<embedded-test.x>     上游那一枚：留住 `.embedded_test` 段，并用
//!                           `PROVIDE(embedded_test_linker_file_not_added_to_rustflags =
//!                           __embedded_test_start)` 兜住 `embedded-test` 那个自检符号
//! ```
//!
//! **为什么必须两枚都在**：`embedded-test` 的 `export.rs` 里有一枚
//! `ensure_linker_file_was_added_to_rustflags()`——它故意引一个**只有链接脚本能给**的符号；
//! 脚本不在场 ⇒ 链接期直接 `undefined symbol:
//! embedded_test_linker_file_not_added_to_rustflags`。
//!
//! # 上游那一枚为什么不借 runner／不借 `DEP_*`
//!
//! - `cargo-qtest`（`cargo-qemu-test`）对 riscv 有一条"**manifest 目录里已经有 `.x` ⇒ 认为你
//!   有自己的脚本、不再注入**"的分支——`kernel/link.x` 正好命中，故它对我们这一景**两枚都不注入**；
//! - `embedded-test-linker-script` 的构建脚本只发 `cargo:rustc-link-search`，**不发
//!   `DEP_…_LINK`** ⇒ 那一格读不到（实测：加了也静默不生效）。
//!
//! 故本脚本走**自包含**那一手：把上游那枚脚本**从它的源文件读出来**，写进本 crate 自己的
//! `OUT_DIR`，再用**绝对路径**交代给测试目标。
//!
//! **哈希自检是故意的**：上游哪天改了那枚脚本，这里当场编不过并印出两条哈希——
//! 逼人看一眼改了哪几行再抄过来，而不是让两份脚本**静默漂移**。
//! 抄的是 `embedded-test-linker-script 0.1.0`（`Cargo.lock` 钉住版本；`env` feature 为默认关，
//! 故没有上游给 `std` 那一档追加的 `INSERT AFTER .comment;`）。
//!
//! **刷新法**：从下面 `UPSTREAM_MARKER` 印出的路径重新读一份，替换 `UPSTREAM` 与 `UPSTREAM_HASH`。

use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// 抄来的那一份（`embedded-test-linker-script 0.1.0`，逐字）。
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

/// 期望的 `sha256(UPSTREAM)`（`sha256sum` 取自上游源文件，见 [`UPSTREAM_MARKER`]）。
const UPSTREAM_HASH: &str = "40c1d2c464932a91c717e13539f4f58716da11f584b84e9719cf1f268ca01b59";

/// 上游那份脚本的**源文件**：拿它来复核哈希（判据落在"真源"上，不落在某一份副本上）。
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
    // 内核链接脚本：workspace 化后不同 crate 用不同脚本（内核 `0x80200000` /
    // 用户 `0x10000`），不能放根 `.cargo/config.toml`（全局 rustflags 冲突），改由本脚本传绝对路径。
    //
    // **（后缀 `ld` → `x`，用户裁定"迁移到 embedded-test"）**：`cargo-qtest` 对 riscv
    // 的判据是"manifest 目录里有没有 `.x` 文件"——有就不自己生成一份 `0x80000000` 的 layout
    // （那会与 `SBI.bin` 撞 ROM 区，实测 `Some ROM regions are overlapping`）。名字保持 `link.x`。
    let ld = format!("{}/link.x", env!("CARGO_MANIFEST_DIR"));
    println!("cargo::rustc-link-arg=-T{ld}");
    println!("cargo::rerun-if-changed=link.x"); // link.x 变更自动重链

    // 上游那一枚：**只给 test 目标**（见文件头）。先自检，再落到本 crate 的 OUT_DIR。
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
            Ok((Some(real_hash), true)) => {}
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
