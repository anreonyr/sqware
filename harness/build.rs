//! harness 的构建脚本：**同一张链接脚本** + **按 bin 生成入口胶水**。
//!
//! 入口那一格与 `programs/build.rs` 是同一件事（同一段代码、同一套名字规矩、同一份生成物
//! 形状），只是这一侧有 21 条 bin。为什么不把它抽成共享函数：两份 `build.rs` 是两次独立的
//! 宿主进程，共享只能靠再引一个 crate——**为二十行代码拖一棵树进来不值**（与那只手写
//! Cargo.toml 解析同一条口径）。改动时记得**两边一起改**。

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    // 与 `programs` **同一张链接脚本**（镜像程序都链在 IMAGE_BASE = 0x10000，见那份
    // `link.ld` 头注）。测具与产品是同一批"被内核装载的 ELF"，链接口径没有第二套。
    let ld = format!("{}/../programs/link.ld", env!("CARGO_MANIFEST_DIR"));
    println!("cargo::rustc-link-arg=-T{ld}");
    println!("cargo::rerun-if-changed={ld}");

    write_entry_shims();
}

/// 读自己的 `Cargo.toml`，按每个 `[[bin]]` 生成一份入口胶水到 `OUT_DIR`。
fn write_entry_shims() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    println!("cargo::rerun-if-changed=Cargo.toml");
    let text = fs::read_to_string(&manifest).expect("读不到 Cargo.toml");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("没有 OUT_DIR"));
    for (bin, path) in bins(&text) {
        // `src/probe_lease.rs` → `entry_probe_lease.rs`；`src/lodger/main.rs` → `entry_lodger_main.rs`
        let ident = path
            .trim_end_matches(".rs")
            .trim_start_matches("src/")
            .replace(['/', '-'], "_");
        let shim = format!(
            r#"// 本文件由 harness/build.rs 生成——**不要手改**（要改改那边）。
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
        if fs::read_to_string(&dst).ok().as_deref() != Some(shim.as_str()) {
            fs::write(&dst, shim).unwrap_or_else(|e| panic!("写不进 {}: {e}", dst.display()));
        }
    }
}

/// 从 `Cargo.toml` 里挑出每个 `[[bin]]` 的 `(name, path)`。
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
