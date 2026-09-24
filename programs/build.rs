//! programs 的构建脚本：**镜像链接脚本** + **按 bin 生成入口胶水**。
//!
//! # 为什么入口胶水要在这里生成（"让写 sqware 像写 std"那一刀）
//!
//! `std` 里那条链是：工具链给 `_start` → rustc 的 `lang_start` 把 `main` 的返回值折成
//! `ExitCode` → `std::process::exit`。**中间那一格是编译期生成的，写程序的人不写**。
//!
//! 本仓（`no_main`）没有 `lang_start`，于是它在这里补上：每个 `[[bin]]` 生成一份
//!
//! ```ignore
//! #[unsafe(no_mangle)]
//! extern "C" fn main() {
//!     let out = main();                       // ← **bin 自己那个 main**（同一模块里）
//!     programs::entry::finish(programs::Exit::report(&out))
//! }
//! ```
//!
//! `#[unsafe(no_mangle)]` 与那个符号名**只出现在生成物里**——写程序的人不签这个合同：
//! `main` 的签名它自己挑（`()` / `Reason` / `Report` / `Result<…>` / `!`），折成退出码那一手
//! 在 `programs::Exit`。各 bin 的 `main.rs` 里只需要接一行生成物：
//!
//! ```ignore
//! include!(concat!(env!("OUT_DIR"), "/entry_driver_uart_main.rs"));
//! ```

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    // 镜像程序固定链接在 IMAGE_BASE (0x10000)，见 link.ld。
    // 本 crate 的每个 [[bin]] 都由内核 `build.rs` 经嵌套 cargo 构建并打进 initrd。
    let ld = format!("{}/link.ld", env!("CARGO_MANIFEST_DIR"));
    println!("cargo::rustc-link-arg=-T{ld}");
    println!("cargo::rerun-if-changed=link.ld");

    write_entry_shims();
}

/// 读自己的 `Cargo.toml`，按每个 `[[bin]]` 生成一份入口胶水到 `OUT_DIR`。
///
/// 三张 `[[bin]]` 表（`programs` 9 条、`harness` 21 条）走的是同一段：`[[bin]]` 一节里
/// `name` 与 `path` 各一行，缺一条就 panic——**不静默跳过**（"没有门的档 = 没有编译过的档"）。
fn write_entry_shims() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    println!("cargo::rerun-if-changed=Cargo.toml");
    let text = fs::read_to_string(&manifest).expect("读不到 Cargo.toml");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("没有 OUT_DIR"));
    for (bin, path) in bins(&text) {
        // `src/driver/uart/main.rs` → `entry_driver_uart_main.rs`
        // （与 `include!` 那一行里的名字同一条规矩：去掉 `src/`、`/` 与 `-` 换 `_`）。
        let ident = path
            .trim_end_matches(".rs")
            .trim_start_matches("src/")
            .replace(['/', '-'], "_");
        let shim = format!(
            r#"// 本文件由 programs/build.rs 生成——**不要手改**（要改改那边）。
//
// `_start` 的汇编调的就是这个 `main` 符号；它把 bin 那个 `main`（`{bin}`）的返回值
// 折成 `Report` 再送进内核。返回类型那一格由 `programs::Exit` 说话。
//
// **为什么裹一层模块**：bin 自己那个 `main` 也叫 `main`，同模块里放不下第二个；而
// `crate::main` 这个路径恰好能从**任何**模块指到 crate 根的那个 `main`——于是这里是
// "在别的模块里看它"，你那边因此不必改名、也不必签任何 attribute。
mod __entry {{
    #[unsafe(no_mangle)]
    extern "C" fn main() {{
        programs::entry::entry(crate::main)
    }}
}}
"#
        );
        let dst = out_dir.join(format!("entry_{ident}.rs"));
        // 内容没变就不写：免得每次 `cargo build` 都让下游重编（门口那几门在乎增量）。
        if fs::read_to_string(&dst).ok().as_deref() != Some(shim.as_str()) {
            fs::write(&dst, shim).unwrap_or_else(|e| panic!("写不进 {}: {e}", dst.display()));
        }
    }
}

/// 从 `Cargo.toml` 里挑出每个 `[[bin]]` 的 `(name, path)`。
///
/// 手写一小段解析、不引 toml 依赖：本 crate 的构建脚本不该为两行字符串拖一棵树进来。
fn bins(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut cur: Option<(Option<String>, Option<String>)> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            if let Some((name, path)) = cur.take() {
                out.push(closed(name, path));
            }
            cur = (line == "[[bin]]").then(|| (None, None));
            continue;
        }
        let Some((name, path)) = cur.as_mut() else {
            continue;
        };
        if line.starts_with('#') {
            continue;
        }
        if let Some(v) = line.strip_prefix("name") {
            *name = quoted(v);
        } else if let Some(v) = line.strip_prefix("path") {
            *path = quoted(v);
        }
    }
    if let Some((name, path)) = cur.take() {
        out.push(closed(name, path));
    }
    out
}

fn closed(name: Option<String>, path: Option<String>) -> (String, String) {
    match (name, path) {
        (Some(n), Some(p)) => (n, p),
        (n, p) => panic!("[[bin]] 少了 name/path：name={n:?} path={p:?}"),
    }
}

/// `name = "prog-echo"` → `Some("prog-echo")`。
fn quoted(rest: &str) -> Option<String> {
    let (_, r) = rest.split_once('=')?;
    let inner = r.trim().strip_prefix('"')?.split('"').next()?;
    Some(inner.to_string())
}
